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

enum JitExport {
    Value(String),
    Null,
    Undefined,
    Object(Vec<(String, JitExport)>),
    Dictionary(Vec<JitDictionaryEntry>),
    Tuple(Vec<JitExport>),
    Conditional(String, Box<JitExport>, Box<JitExport>),
    WithLocals(Vec<JitLocal>, Box<JitExport>),
}

enum JitDictionaryEntry {
    Static(String, JitExport),
    Computed(JitExport, JitExport),
    Spread(JitExport),
}

struct JitLocal {
    ty: thaw_hir::HirType,
    operation: String,
}

fn jit_tagged_union(elements: &[thaw_hir::HirType]) -> bool {
    jit_argument_tagged_union(elements)
}

fn jit_argument_tagged_union(elements: &[thaw_hir::HirType]) -> bool {
    (2..=9).contains(&elements.len())
        && elements.iter().all(|element| jit_union_member_code(element).is_some())
        && elements
            .iter()
            .enumerate()
            .all(|(index, element)| !elements[..index].contains(element))
        && elements
            .iter()
            .filter(|element| matches!(element, thaw_hir::HirType::Object(_)))
            .count()
            <= 1
        && elements
            .iter()
            .filter(|element| matches!(element, thaw_hir::HirType::Tuple(_)))
            .count()
            <= 1
        && (elements.contains(&thaw_hir::HirType::Str)
            || elements.iter().any(|element| {
                matches!(
                    element,
                    thaw_hir::HirType::Array(_)
                        | thaw_hir::HirType::Dictionary(_)
                        | thaw_hir::HirType::Object(_)
                        | thaw_hir::HirType::Tuple(_)
                )
            }))
}

fn jit_union_member_code(ty: &thaw_hir::HirType) -> Option<char> {
    match ty {
        thaw_hir::HirType::F64 => Some('n'),
        thaw_hir::HirType::Bool => Some('b'),
        thaw_hir::HirType::Str => Some('s'),
        thaw_hir::HirType::Array(element) => match element.as_ref() {
            thaw_hir::HirType::F64 => Some('N'),
            thaw_hir::HirType::Bool => Some('B'),
            thaw_hir::HirType::Str => Some('S'),
            _ => None,
        },
        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
            thaw_hir::HirType::F64 => Some('D'),
            thaw_hir::HirType::Bool => Some('E'),
            thaw_hir::HirType::Str => Some('F'),
            _ => None,
        },
        thaw_hir::HirType::Object(_) => Some('O'),
        thaw_hir::HirType::Tuple(elements) => {
            if !elements.is_empty() && elements.iter().all(|element| *element == thaw_hir::HirType::F64) {
                Some('X')
            } else if !elements.is_empty()
                && elements.iter().all(|element| *element == thaw_hir::HirType::Bool)
            {
                Some('Y')
            } else if !elements.is_empty()
                && elements.iter().all(|element| *element == thaw_hir::HirType::Str)
            {
                Some('Z')
            } else {
                Some('T')
            }
        }
        _ => None,
    }
}

fn is_unary_math_method(operation: &str) -> bool {
    matches!(
        operation,
        "abs"
            | "acos"
            | "acosh"
            | "asin"
            | "asinh"
            | "atan"
            | "atanh"
            | "cbrt"
            | "ceil"
            | "clz32"
            | "cos"
            | "cosh"
            | "exp"
            | "expm1"
            | "floor"
            | "fround"
            | "log"
            | "log1p"
            | "log2"
            | "log10"
            | "round"
            | "sign"
            | "sin"
            | "sinh"
            | "sqrt"
            | "tan"
            | "tanh"
            | "trunc"
    )
}

fn jit_export(
    source: &str,
    export_name: &str,
    allow_default: bool,
    function: &thaw_bridge::DtsFunction,
) -> Option<JitExport> {
    use thaw_parser::ast::{
        ArrowExpr, AssignExpr, AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, Decl, Expr,
        ExprOrSpread, ForHead, Function, Ident, Lit, MemberProp, ModuleItem, OptChainBase, Pat,
        Prop, PropName, PropOrSpread, SimpleAssignTarget, Stmt, UnaryOp, UpdateOp, VarDeclKind,
        VarDeclOrExpr,
    };

    fn jit_parameter_slots(ty: &thaw_hir::HirType) -> Option<usize> {
        match ty {
            thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str => Some(1),
            thaw_hir::HirType::Union(elements) if jit_argument_tagged_union(elements) => Some(2),
            thaw_hir::HirType::Array(element)
                if jit_array_result_element_supported(element) =>
            {
                Some(1)
            }
            thaw_hir::HirType::Dictionary(element)
                if matches!(
                    element.as_ref(),
                    thaw_hir::HirType::F64
                        | thaw_hir::HirType::Bool
                        | thaw_hir::HirType::Str
                ) =>
            {
                Some(1)
            }
            thaw_hir::HirType::Object(fields) => fields.iter().try_fold(0usize, |slots, (_, ty)| {
                jit_parameter_slots(ty).map(|count| slots + count)
            }),
            thaw_hir::HirType::Tuple(elements) => elements.iter().try_fold(0usize, |slots, ty| {
                jit_parameter_slots(ty).map(|count| slots + count)
            }),
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload) => {
                jit_parameter_slots(payload).map(|slots| slots + 1)
            }
            _ => None,
        }
    }

    fn jit_array_result_element_supported(ty: &thaw_hir::HirType) -> bool {
        matches!(
            ty,
            thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str
        ) || matches!(
            ty,
            thaw_hir::HirType::Tuple(elements)
                if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str])
        )
    }

    fn jit_result_supported(ty: &thaw_hir::HirType) -> bool {
        match ty {
            thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str => true,
            thaw_hir::HirType::Union(elements) => {
                jit_tagged_union(elements)
            }
            thaw_hir::HirType::Array(element) => jit_array_result_element_supported(element),
            thaw_hir::HirType::Dictionary(element) => matches!(
                element.as_ref(),
                thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str
            ),
            thaw_hir::HirType::Object(fields) => {
                fields.iter().all(|(_, ty)| jit_result_supported(ty))
            }
            thaw_hir::HirType::Tuple(elements) => elements.iter().all(jit_result_supported),
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload) => {
                jit_result_supported(payload)
            }
            _ => false,
        }
    }

    fn bind_jit_aggregate_fields(
        path: &str,
        ty: &thaw_hir::HirType,
        parameters: &mut std::collections::HashMap<String, String>,
        slot: &mut usize,
    ) -> Option<()> {
        if let thaw_hir::HirType::Union(elements) = ty {
            if !jit_argument_tagged_union(elements) {
                return None;
            }
            let kinds = elements
                .iter()
                .map(jit_union_member_code)
                .collect::<Option<String>>()?;
            parameters.insert(path.into(), format!("u{}{kinds}", *slot));
            *slot += 2;
            return Some(());
        }
        let prefix = match ty {
            thaw_hir::HirType::Str => Some("s"),
            thaw_hir::HirType::Bool => Some("b"),
            thaw_hir::HirType::F64 => Some("a"),
            thaw_hir::HirType::Array(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => Some("rn"),
                thaw_hir::HirType::Bool => Some("rb"),
                thaw_hir::HirType::Str => Some("rs"),
                _ => None,
            },
            thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => Some("dn"),
                thaw_hir::HirType::Bool => Some("db"),
                thaw_hir::HirType::Str => Some("ds"),
                _ => None,
            },
            thaw_hir::HirType::Object(fields) => {
                for (field, field_type) in fields {
                    bind_jit_aggregate_fields(
                        &format!("{path}.{field}"),
                        field_type,
                        parameters,
                        slot,
                    )?;
                }
                return Some(());
            }
            thaw_hir::HirType::Tuple(elements) => {
                for (index, element) in elements.iter().enumerate() {
                    bind_jit_aggregate_fields(
                        &format!("{path}.{index}"),
                        element,
                        parameters,
                        slot,
                    )?;
                }
                return Some(());
            }
            _ => None,
        }?;
        parameters.insert(path.into(), format!("{prefix}{slot}"));
        *slot += 1;
        Some(())
    }

    fn bind_jit_union_object_fields(
        path: &str,
        elements: &[thaw_hir::HirType],
        source: &[String],
        locals: &mut std::collections::HashMap<String, Vec<String>>,
    ) -> Option<()> {
        if !elements.iter().any(|element| {
            matches!(
                element,
                thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)
            )
        }) {
            return Some(());
        }
        fn bind_fields(
            path: &str,
            fields: &[(String, thaw_hir::HirType)],
            source: &[String],
            locals: &mut std::collections::HashMap<String, Vec<String>>,
        ) -> Option<()> {
            let mut offset = 0u16;
            for (field, ty) in fields {
                let operation = match ty {
                    thaw_hir::HirType::F64 => Some("objn"),
                    thaw_hir::HirType::Bool => Some("objb"),
                    thaw_hir::HirType::Str => Some("objs"),
                    thaw_hir::HirType::Array(element)
                        if jit_array_result_element_supported(element) =>
                    {
                        Some(match element.as_ref() {
                            thaw_hir::HirType::F64 => "objrn",
                            thaw_hir::HirType::Bool => "objrb",
                            thaw_hir::HirType::Str => "objrs",
                            _ => unreachable!(),
                        })
                    }
                    thaw_hir::HirType::Object(_) => Some("objo"),
                    thaw_hir::HirType::Dictionary(element) => Some(match element.as_ref() {
                        thaw_hir::HirType::F64 => "objdn",
                        thaw_hir::HirType::Bool => "objdb",
                        thaw_hir::HirType::Str => "objds",
                        _ => return None,
                    }),
                    thaw_hir::HirType::Tuple(_) => Some("objt"),
                    thaw_hir::HirType::Optional(payload) => Some(match payload.as_ref() {
                        thaw_hir::HirType::F64 => "objoptn",
                        thaw_hir::HirType::Bool => "objoptb",
                        thaw_hir::HirType::Str => "objopts",
                        thaw_hir::HirType::Object(_) => "objopto",
                        thaw_hir::HirType::Tuple(_) => "objoptt",
                        thaw_hir::HirType::Array(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objoptrn",
                            thaw_hir::HirType::Bool => "objoptrb",
                            thaw_hir::HirType::Str => "objoptrs",
                            _ => return None,
                        },
                        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objoptdn",
                            thaw_hir::HirType::Bool => "objoptdb",
                            thaw_hir::HirType::Str => "objoptds",
                            _ => return None,
                        },
                        _ => return None,
                    }),
                    thaw_hir::HirType::Nullable(payload) => Some(match payload.as_ref() {
                        thaw_hir::HirType::F64 => "objnullablen",
                        thaw_hir::HirType::Bool => "objoptb",
                        thaw_hir::HirType::Str => "objopts",
                        thaw_hir::HirType::Object(_) => "objopto",
                        thaw_hir::HirType::Tuple(_) => "objoptt",
                        thaw_hir::HirType::Array(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objoptrn",
                            thaw_hir::HirType::Bool => "objoptrb",
                            thaw_hir::HirType::Str => "objoptrs",
                            _ => return None,
                        },
                        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objoptdn",
                            thaw_hir::HirType::Bool => "objoptdb",
                            thaw_hir::HirType::Str => "objoptds",
                            _ => return None,
                        },
                        _ => return None,
                    }),
                    thaw_hir::HirType::Nullish(payload) => Some(match payload.as_ref() {
                        thaw_hir::HirType::F64 => "objnulln",
                        thaw_hir::HirType::Bool => "objnullb",
                        thaw_hir::HirType::Str => "objnulls",
                        thaw_hir::HirType::Object(_) => "objnullo",
                        thaw_hir::HirType::Tuple(_) => "objnullt",
                        thaw_hir::HirType::Array(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objnullrn",
                            thaw_hir::HirType::Bool => "objnullrb",
                            thaw_hir::HirType::Str => "objnullrs",
                            _ => return None,
                        },
                        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "objnulldn",
                            thaw_hir::HirType::Bool => "objnulldb",
                            thaw_hir::HirType::Str => "objnullds",
                            _ => return None,
                        },
                        _ => return None,
                    }),
                    _ => None,
                };
                if let Some(operation) = operation {
                    let field_path = format!("{path}.{field}");
                    let mut value = source.to_vec();
                    value.push(format!("{operation}{offset}"));
                    locals.insert(field_path.clone(), value.clone());
                    if let thaw_hir::HirType::Object(nested) = ty {
                        bind_fields(&field_path, nested, &value, locals)?;
                    } else if let thaw_hir::HirType::Tuple(types) = ty {
                        bind_tuple(&field_path, types, &value, locals)?;
                    } else if let thaw_hir::HirType::Optional(payload)
                    | thaw_hir::HirType::Nullable(payload)
                    | thaw_hir::HirType::Nullish(payload) = ty
                    {
                        match payload.as_ref() {
                            thaw_hir::HirType::Object(nested) => {
                                bind_fields(&field_path, nested, &value, locals)?
                            }
                            thaw_hir::HirType::Tuple(types) => {
                                bind_tuple(&field_path, types, &value, locals)?
                            }
                            _ => {}
                        }
                    }
                }
                offset = offset.checked_add(if matches!(
                    ty,
                    thaw_hir::HirType::Optional(_)
                        | thaw_hir::HirType::Nullable(_)
                        | thaw_hir::HirType::Nullish(_)
                        | thaw_hir::HirType::Union(_)
                ) {
                    16
                } else {
                    8
                })?;
            }
            Some(())
        }
        fn bind_tuple(
            path: &str,
            types: &[thaw_hir::HirType],
            source: &[String],
            locals: &mut std::collections::HashMap<String, Vec<String>>,
        ) -> Option<()> {
            for (index, element) in types.iter().enumerate() {
                let operation = match element {
                    thaw_hir::HirType::F64 => "rnget",
                    thaw_hir::HirType::Bool => "rbget",
                    thaw_hir::HirType::Str => "rsget",
                    thaw_hir::HirType::Tuple(_) => "raget",
                    thaw_hir::HirType::Object(_) => "roget",
                    thaw_hir::HirType::Array(element) => match element.as_ref() {
                        thaw_hir::HirType::F64 => "ragetrn",
                        thaw_hir::HirType::Bool => "ragetrb",
                        thaw_hir::HirType::Str => "ragetrs",
                        _ => return None,
                    },
                    thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                        thaw_hir::HirType::F64 => "rogetdn",
                        thaw_hir::HirType::Bool => "rogetdb",
                        thaw_hir::HirType::Str => "rogetds",
                        _ => return None,
                    },
                    thaw_hir::HirType::Optional(payload)
                    | thaw_hir::HirType::Nullable(payload) => match payload.as_ref() {
                        thaw_hir::HirType::F64 => "tupoptn",
                        thaw_hir::HirType::Bool => "tupoptb",
                        thaw_hir::HirType::Str => "tupopts",
                        thaw_hir::HirType::Object(_) => "tupopto",
                        thaw_hir::HirType::Tuple(_) => "tupoptt",
                        thaw_hir::HirType::Array(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "tupoptrn",
                            thaw_hir::HirType::Bool => "tupoptrb",
                            thaw_hir::HirType::Str => "tupoptrs",
                            _ => return None,
                        },
                        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "tupoptdn",
                            thaw_hir::HirType::Bool => "tupoptdb",
                            thaw_hir::HirType::Str => "tupoptds",
                            _ => return None,
                        },
                        _ => return None,
                    },
                    thaw_hir::HirType::Nullish(payload) => match payload.as_ref() {
                        thaw_hir::HirType::F64 => "tupnulln",
                        thaw_hir::HirType::Bool => "tupnullb",
                        thaw_hir::HirType::Str => "tupnulls",
                        thaw_hir::HirType::Object(_) => "tupnullo",
                        thaw_hir::HirType::Tuple(_) => "tupnullt",
                        thaw_hir::HirType::Array(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "tupnullrn",
                            thaw_hir::HirType::Bool => "tupnullrb",
                            thaw_hir::HirType::Str => "tupnullrs",
                            _ => return None,
                        },
                        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                            thaw_hir::HirType::F64 => "tupnulldn",
                            thaw_hir::HirType::Bool => "tupnulldb",
                            thaw_hir::HirType::Str => "tupnullds",
                            _ => return None,
                        },
                        _ => return None,
                    },
                    _ => return None,
                };
                let element_path = format!("{path}.{index}");
                let mut value = source.to_vec();
                if matches!(
                    element,
                    thaw_hir::HirType::Optional(_)
                        | thaw_hir::HirType::Nullable(_)
                        | thaw_hir::HirType::Nullish(_)
                ) {
                    value.push(format!("{operation}{index}"));
                } else {
                    value.push(format!("c{:016x}", (index as f64).to_bits()));
                    value.push(operation.into());
                }
                locals.insert(element_path.clone(), value.clone());
                if let thaw_hir::HirType::Tuple(types) = element {
                    bind_tuple(&element_path, types, &value, locals)?;
                } else if let thaw_hir::HirType::Object(fields) = element {
                    bind_fields(&element_path, fields, &value, locals)?;
                } else if let thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload) = element
                {
                    match payload.as_ref() {
                        thaw_hir::HirType::Tuple(types) => {
                            bind_tuple(&element_path, types, &value, locals)?
                        }
                        thaw_hir::HirType::Object(fields) => {
                            bind_fields(&element_path, fields, &value, locals)?
                        }
                        _ => {}
                    }
                }
            }
            Some(())
        }
        for aggregate in elements {
            let mut object = source.to_vec();
            match aggregate {
                thaw_hir::HirType::Object(fields) => {
                    object.push("untagobject".into());
                    bind_fields(path, fields, &object, locals)?;
                }
                thaw_hir::HirType::Tuple(types) => {
                    object.push("untagtuple".into());
                    bind_tuple(path, types, &object, locals)?;
                }
                _ => {}
            }
        }
        Some(())
    }

    fn member_path(expression: &Expr) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => Some(identifier.sym.to_string()),
            Expr::Member(member) => {
                let property = match &member.prop {
                    MemberProp::Ident(property) => property.sym.to_string(),
                    MemberProp::Computed(computed) => {
                        match computed.expr.as_ref() {
                            Expr::Lit(Lit::Num(index))
                                if index.value >= 0.0 && index.value.fract() == 0.0 =>
                            {
                                (index.value as usize).to_string()
                            }
                            Expr::Lit(Lit::Str(property)) => {
                                property.value.to_string_lossy().into_owned()
                            }
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                Some(format!("{}.{}", member_path(member.obj.as_ref())?, property))
            }
            _ => None,
        }
    }

    fn object_literal(expression: &Expr) -> Option<&thaw_parser::ast::ObjectLit> {
        match expression {
            Expr::Object(object) => Some(object),
            Expr::Paren(parenthesized) => object_literal(parenthesized.expr.as_ref()),
            _ => None,
        }
    }

    fn encode_aggregate_body(
        body: NumericBody<'_>,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        match body {
            NumericBody::Expression(expression) => {
                encode_return_expression(expression, ty, parameters, locals, context)
            }
            NumericBody::Statements(statements) => {
                encode_aggregate_statements(statements, ty, parameters, locals, context)
            }
        }
    }

    fn encode_aggregate_statement(
        statement: &Stmt,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        match statement {
            Stmt::Return(returned) => encode_return_expression(
                returned.arg.as_deref()?,
                ty,
                parameters,
                locals,
                context,
            ),
            Stmt::Block(block) => {
                encode_aggregate_statements(&block.stmts, ty, parameters, locals, context)
            }
            Stmt::If(_) => encode_aggregate_statements(
                std::slice::from_ref(statement),
                ty,
                parameters,
                locals,
                context,
            ),
            Stmt::Switch(switch) => {
                let mut encoded = Vec::new();
                encode_returning_switch(switch, parameters, locals, context, &mut encoded)?;
                Some(JitExport::Value(validated_jit_expression(
                    encoded,
                    jit_return_kind(ty)?,
                )?))
            }
            _ => None,
        }
    }

    fn encode_aggregate_statements(
        statements: &[Stmt],
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        let (first, rest) = statements.split_first()?;
        if let Stmt::Return(_) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_aggregate_statement(first, ty, parameters, locals, context);
        }
        if let Stmt::Switch(_) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_aggregate_statement(first, ty, parameters, locals, context);
        }
        let Stmt::If(branch) = first else {
            return None;
        };
        let mut condition = Vec::new();
        encode_condition(
            branch.test.as_ref(),
            parameters,
            locals,
            context,
            &mut condition,
        )?;
        let narrowed = narrowed_locals(
            branch.test.as_ref(),
            parameters,
            locals,
            context.helpers,
        );
        let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
        let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
        let consequent = encode_aggregate_statement(
            branch.cons.as_ref(),
            ty,
            parameters,
            consequent_locals,
            context,
        )?;
        let alternate = if let Some(alternate) = branch.alt.as_deref() {
            if !rest.is_empty() {
                return None;
            }
            encode_aggregate_statement(alternate, ty, parameters, alternate_locals, context)?
        } else {
            encode_aggregate_statements(rest, ty, parameters, alternate_locals, context)?
        };
        Some(JitExport::Conditional(
            validated_jit_expression(condition, JitKind::Boolean)?,
            Box::new(consequent),
            Box::new(alternate),
        ))
    }

    enum ObjectReturnValue<'a> {
        Expression(&'a Expr),
        Shorthand(&'a Ident),
    }

    fn object_property(property: &PropOrSpread) -> Option<(String, ObjectReturnValue<'_>)> {
        let PropOrSpread::Prop(property) = property else {
            return None;
        };
        match property.as_ref() {
            Prop::KeyValue(property) => {
                let name = match &property.key {
                    PropName::Ident(identifier) => identifier.sym.to_string(),
                    PropName::Str(string) => string.value.to_string_lossy().into_owned(),
                    _ => return None,
                };
                Some((name, ObjectReturnValue::Expression(property.value.as_ref())))
            }
            Prop::Shorthand(identifier) => Some((
                identifier.sym.to_string(),
                ObjectReturnValue::Shorthand(identifier),
            )),
            _ => None,
        }
    }

    fn encode_object_return(
        object: &thaw_parser::ast::ObjectLit,
        fields: &[(String, thaw_hir::HirType)],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        if object.props.len() > fields.len() {
            return None;
        }
        let mut names = std::collections::HashSet::new();
        object
            .props
            .iter()
            .map(|property| {
                let (name, expression) = object_property(property)?;
                if !names.insert(name.clone()) {
                    return None;
                }
                let ty = &fields.iter().find(|(field, _)| field == &name)?.1;
                let value = match expression {
                    ObjectReturnValue::Expression(expression) => encode_return_expression(
                        expression,
                        ty,
                        parameters,
                        locals,
                        context,
                    )?,
                    ObjectReturnValue::Shorthand(identifier) => {
                        let expected = jit_return_kind(ty)?;
                        let encoded = if let Some(value) = locals.get(identifier.sym.as_ref()) {
                            value.clone()
                        } else {
                            vec![parameters.get(identifier.sym.as_ref())?.clone()]
                        };
                        JitExport::Value(validated_jit_expression(encoded, expected)?)
                    }
                };
                Some((name, value))
            })
            .collect::<Option<Vec<_>>>()
            .map(JitExport::Object)
    }

    fn jit_kind_compatible(actual: JitKind, expected: JitKind) -> bool {
        actual == expected
            || (matches!(actual, JitKind::Number | JitKind::Boolean)
                && matches!(expected, JitKind::Number | JitKind::Boolean))
    }

    fn encode_fixed_tuple_value(
        expression: &Expr,
        types: &[thaw_hir::HirType],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<Vec<String>> {
        let Expr::Array(tuple) = expression else {
            return None;
        };
        if tuple.elems.len() != types.len() {
            return None;
        }
        let length = u16::try_from(types.len()).ok()?;
        let wide = types.iter().any(|ty| {
            matches!(
                ty,
                thaw_hir::HirType::Optional(_)
                    | thaw_hir::HirType::Nullable(_)
                    | thaw_hir::HirType::Nullish(_)
                    | thaw_hir::HirType::Union(_)
            )
        });
        let mut output = vec![format!("tupnew{}{length}", if wide { "w" } else { "" })];
        for (index, (element, ty)) in tuple.elems.iter().zip(types).enumerate() {
            let element = element.as_ref()?;
            if element.spread.is_some() {
                return None;
            }
            let (value, kind, mode) = match ty {
                thaw_hir::HirType::Nullish(payload) => {
                    let absent_tag = if matches!(element.expr.as_ref(), Expr::Lit(Lit::Null(_))) {
                        Some(1.0f64)
                    } else if matches!(element.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == "undefined") {
                        Some(2.0f64)
                    } else {
                        None
                    };
                    if let Some(tag) = absent_tag {
                        output.push(format!("c{:016x}", tag.to_bits()));
                        output.push(format!("tupsetwu{index}"));
                        continue;
                    }
                    let (value, kind) = match payload.as_ref() {
                        thaw_hir::HirType::Object(fields) => (
                            encode_fixed_object_value(
                                object_literal(element.expr.as_ref())?,
                                fields,
                                parameters,
                                locals,
                                context,
                            )?,
                            'o',
                        ),
                        thaw_hir::HirType::Tuple(types) => (
                            encode_fixed_tuple_value(
                                element.expr.as_ref(),
                                types,
                                parameters,
                                locals,
                                context,
                            )?,
                            'p',
                        ),
                        thaw_hir::HirType::Array(element_type)
                            if jit_array_result_element_supported(element_type) =>
                        {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            if jit_expression_kind(&value)?.0 != JitKind::Array {
                                return None;
                            }
                            value.push("arrayhandle".into());
                            (value, 'p')
                        }
                        thaw_hir::HirType::Dictionary(element_type)
                            if matches!(
                                element_type.as_ref(),
                                thaw_hir::HirType::F64
                                    | thaw_hir::HirType::Bool
                                    | thaw_hir::HirType::Str
                            ) =>
                        {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                                return None;
                            }
                            (value, 'o')
                        }
                        primitive => {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            let expected = jit_return_kind(primitive)?;
                            if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                                return None;
                            }
                            let kind = match primitive {
                                thaw_hir::HirType::F64 => 'n',
                                thaw_hir::HirType::Bool => 'b',
                                thaw_hir::HirType::Str => 's',
                                _ => return None,
                            };
                            (value, kind)
                        }
                    };
                    (value, kind, 2)
                }
                thaw_hir::HirType::Optional(payload) | thaw_hir::HirType::Nullable(payload) => {
                    if matches!(element.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == "undefined")
                        || matches!(element.expr.as_ref(), Expr::Lit(Lit::Null(_)))
                    {
                        continue;
                    }
                    let (value, kind) = match payload.as_ref() {
                        thaw_hir::HirType::Object(fields) => (
                            encode_fixed_object_value(
                                object_literal(element.expr.as_ref())?,
                                fields,
                                parameters,
                                locals,
                                context,
                            )?,
                            'o',
                        ),
                        thaw_hir::HirType::Tuple(types) => (
                            encode_fixed_tuple_value(
                                element.expr.as_ref(),
                                types,
                                parameters,
                                locals,
                                context,
                            )?,
                            'p',
                        ),
                        thaw_hir::HirType::Array(element_type)
                            if jit_array_result_element_supported(element_type) =>
                        {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            if jit_expression_kind(&value)?.0 != JitKind::Array {
                                return None;
                            }
                            value.push("arrayhandle".into());
                            (value, 'p')
                        }
                        thaw_hir::HirType::Dictionary(element_type)
                            if matches!(
                                element_type.as_ref(),
                                thaw_hir::HirType::F64
                                    | thaw_hir::HirType::Bool
                                    | thaw_hir::HirType::Str
                            ) =>
                        {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                                return None;
                            }
                            (value, 'o')
                        }
                        primitive => {
                            let mut value = Vec::new();
                            encode_expression(
                                element.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut value,
                            )?;
                            let expected = jit_return_kind(primitive)?;
                            if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                                return None;
                            }
                            let kind = match primitive {
                                thaw_hir::HirType::F64 => 'n',
                                thaw_hir::HirType::Bool => 'b',
                                thaw_hir::HirType::Str => 's',
                                _ => return None,
                            };
                            (value, kind)
                        }
                    };
                    (value, kind, 1)
                }
                thaw_hir::HirType::Tuple(types) => (
                    encode_fixed_tuple_value(
                        element.expr.as_ref(),
                        types,
                        parameters,
                        locals,
                        context,
                    )?,
                    'p',
                    0,
                ),
                thaw_hir::HirType::Object(fields) => (
                    encode_fixed_object_value(
                        object_literal(element.expr.as_ref())?,
                        fields,
                        parameters,
                        locals,
                        context,
                    )?,
                    'o',
                    0,
                ),
                thaw_hir::HirType::Array(element_type)
                    if jit_array_result_element_supported(element_type) =>
                {
                    let mut value = Vec::new();
                    encode_expression(
                        element.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut value,
                    )?;
                    if jit_expression_kind(&value)?.0 != JitKind::Array {
                        return None;
                    }
                    value.push("arrayhandle".into());
                    (value, 'p', 0)
                }
                thaw_hir::HirType::Dictionary(element_type)
                    if matches!(
                        element_type.as_ref(),
                        thaw_hir::HirType::F64
                            | thaw_hir::HirType::Bool
                            | thaw_hir::HirType::Str
                    ) =>
                {
                    let mut value = Vec::new();
                    encode_expression(
                        element.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut value,
                    )?;
                    if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                        return None;
                    }
                    (value, 'o', 0)
                }
                _ => {
                    let mut value = Vec::new();
                    encode_expression(
                        element.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut value,
                    )?;
                    let expected = jit_return_kind(ty)?;
                    if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                        return None;
                    }
                    let kind = match ty {
                        thaw_hir::HirType::F64 => 'n',
                        thaw_hir::HirType::Bool => 'b',
                        thaw_hir::HirType::Str => 's',
                        _ => return None,
                    };
                    (value, kind, 0)
                }
            };
            output.extend(value);
            output.push(if mode == 1 {
                format!("tupsetopt{kind}{index}")
            } else if mode == 2 {
                format!("tupsetnull{kind}{index}")
            } else if wide {
                format!("tupsetw{kind}{index}")
            } else {
                format!("tupset{kind}{index}")
            });
        }
        Some(output)
    }

    fn encode_fixed_object_value(
        object: &thaw_parser::ast::ObjectLit,
        fields: &[(String, thaw_hir::HirType)],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<Vec<String>> {
        if object.props.len() > fields.len() {
            return None;
        }
        let mut properties = std::collections::HashMap::new();
        for property in &object.props {
            let (name, value) = object_property(property)?;
            if properties.insert(name, value).is_some() {
                return None;
            }
        }
        if properties
            .keys()
            .any(|property| !fields.iter().any(|(field, _)| field == property))
        {
            return None;
        }
        let field_size = |ty: &thaw_hir::HirType| {
            if matches!(
                ty,
                thaw_hir::HirType::Optional(_)
                    | thaw_hir::HirType::Nullable(_)
                    | thaw_hir::HirType::Nullish(_)
                    | thaw_hir::HirType::Union(_)
            ) {
                16usize
            } else {
                8
            }
        };
        let size = u16::try_from(fields.iter().map(|(_, ty)| field_size(ty)).sum::<usize>()).ok()?;
        let mut output = vec![format!("objnew{size}")];
        let mut offset = 0usize;
        for (field, ty) in fields {
            let Some(property) = properties.get(field) else {
                if matches!(ty, thaw_hir::HirType::Optional(_)) {
                    offset += field_size(ty);
                    continue;
                }
                return None;
            };
            let (value, operation) = match property {
                ObjectReturnValue::Expression(expression) => match ty {
                    thaw_hir::HirType::Object(nested) => (
                        encode_fixed_object_value(
                            object_literal(expression)?,
                            nested,
                            parameters,
                            locals,
                            context,
                        )?,
                        'o',
                    ),
                    thaw_hir::HirType::Array(element)
                        if jit_array_result_element_supported(element) =>
                    {
                        let mut value = Vec::new();
                        encode_expression(expression, parameters, locals, context, &mut value)?;
                        if jit_expression_kind(&value)?.0 != JitKind::Array {
                            return None;
                        }
                        value.push("arrayhandle".into());
                        (value, 'a')
                    }
                    thaw_hir::HirType::Dictionary(element)
                        if matches!(
                            element.as_ref(),
                            thaw_hir::HirType::F64
                                | thaw_hir::HirType::Bool
                                | thaw_hir::HirType::Str
                        ) =>
                    {
                        let mut value = Vec::new();
                        encode_expression(expression, parameters, locals, context, &mut value)?;
                        if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                            return None;
                        }
                        (value, 'o')
                    }
                    thaw_hir::HirType::Tuple(types) => (
                        encode_fixed_tuple_value(
                            expression,
                            types,
                            parameters,
                            locals,
                            context,
                        )?,
                        'a',
                    ),
                    thaw_hir::HirType::Nullish(payload) => {
                        let absent_tag = if matches!(expression, Expr::Lit(Lit::Null(_))) {
                            Some(1.0f64)
                        } else if matches!(expression, Expr::Ident(identifier) if identifier.sym == "undefined") {
                            Some(2.0f64)
                        } else {
                            None
                        };
                        if let Some(tag) = absent_tag {
                            output.push(format!("c{:016x}", tag.to_bits()));
                            output.push(format!("objsetu{offset}"));
                            offset += field_size(ty);
                            continue;
                        }
                        match payload.as_ref() {
                            thaw_hir::HirType::Object(fields) => (
                                encode_fixed_object_value(
                                    object_literal(expression)?,
                                    fields,
                                    parameters,
                                    locals,
                                    context,
                                )?,
                                'o',
                            ),
                            thaw_hir::HirType::Tuple(types) => (
                                encode_fixed_tuple_value(
                                    expression, types, parameters, locals, context,
                                )?,
                                'a',
                            ),
                            thaw_hir::HirType::Array(element_type)
                                if jit_array_result_element_supported(element_type) =>
                            {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                if jit_expression_kind(&value)?.0 != JitKind::Array {
                                    return None;
                                }
                                value.push("arrayhandle".into());
                                (value, 'a')
                            }
                            thaw_hir::HirType::Dictionary(element_type)
                                if matches!(
                                    element_type.as_ref(),
                                    thaw_hir::HirType::F64
                                        | thaw_hir::HirType::Bool
                                        | thaw_hir::HirType::Str
                                ) =>
                            {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                                    return None;
                                }
                                (value, 'o')
                            }
                            primitive => {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                let expected = jit_return_kind(primitive)?;
                                if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                                    return None;
                                }
                                (
                                    value,
                                    match primitive {
                                        thaw_hir::HirType::F64 => 'n',
                                        thaw_hir::HirType::Bool => 'b',
                                        thaw_hir::HirType::Str => 's',
                                        _ => return None,
                                    },
                                )
                            }
                        }
                    }
                    thaw_hir::HirType::Optional(payload)
                    | thaw_hir::HirType::Nullable(payload) => {
                        if matches!(expression, Expr::Lit(Lit::Null(_))) {
                            offset += field_size(ty);
                            continue;
                        }
                        match payload.as_ref() {
                            thaw_hir::HirType::Object(fields) => (
                                encode_fixed_object_value(
                                    object_literal(expression)?,
                                    fields,
                                    parameters,
                                    locals,
                                    context,
                                )?,
                                'o',
                            ),
                            thaw_hir::HirType::Tuple(types) => (
                                encode_fixed_tuple_value(
                                    expression, types, parameters, locals, context,
                                )?,
                                'a',
                            ),
                            thaw_hir::HirType::Array(element_type)
                                if jit_array_result_element_supported(element_type) =>
                            {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                if jit_expression_kind(&value)?.0 != JitKind::Array {
                                    return None;
                                }
                                value.push("arrayhandle".into());
                                (value, 'a')
                            }
                            thaw_hir::HirType::Dictionary(element_type)
                                if matches!(
                                    element_type.as_ref(),
                                    thaw_hir::HirType::F64
                                        | thaw_hir::HirType::Bool
                                        | thaw_hir::HirType::Str
                                ) =>
                            {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                if jit_expression_kind(&value)?.0 != JitKind::Dictionary {
                                    return None;
                                }
                                (value, 'o')
                            }
                            primitive => {
                                let mut value = Vec::new();
                                encode_expression(
                                    expression,
                                    parameters,
                                    locals,
                                    context,
                                    &mut value,
                                )?;
                                let expected = jit_return_kind(primitive)?;
                                if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                                    return None;
                                }
                                (
                                    value,
                                    match primitive {
                                        thaw_hir::HirType::F64 => 'n',
                                        thaw_hir::HirType::Bool => 'b',
                                        thaw_hir::HirType::Str => 's',
                                        _ => return None,
                                    },
                                )
                            }
                        }
                    }
                    _ => {
                        let mut value = Vec::new();
                        encode_expression(expression, parameters, locals, context, &mut value)?;
                        let expected = jit_return_kind(ty)?;
                        if !jit_kind_compatible(jit_expression_kind(&value)?.0, expected) {
                            return None;
                        }
                        (
                            value,
                            match ty {
                                thaw_hir::HirType::F64 => 'n',
                                thaw_hir::HirType::Bool => 'b',
                                thaw_hir::HirType::Str => 's',
                                _ => return None,
                            },
                        )
                    }
                },
                ObjectReturnValue::Shorthand(identifier) => {
                    let value = locals
                        .get(identifier.sym.as_ref())
                        .cloned()
                        .or_else(|| {
                            parameters
                                .get(identifier.sym.as_ref())
                                .map(|value| vec![value.clone()])
                        })?;
                    let expected = jit_return_kind(ty)?;
                    let actual = jit_expression_kind(&value)?.0;
                    if !jit_kind_compatible(actual, expected) {
                        return None;
                    }
                    let operation = match ty {
                        thaw_hir::HirType::F64 => 'n',
                        thaw_hir::HirType::Bool => 'b',
                        thaw_hir::HirType::Str => 's',
                        thaw_hir::HirType::Array(element)
                            if jit_array_result_element_supported(element) =>
                        {
                            'a'
                        }
                        thaw_hir::HirType::Dictionary(element)
                            if matches!(
                                element.as_ref(),
                                thaw_hir::HirType::F64
                                    | thaw_hir::HirType::Bool
                                    | thaw_hir::HirType::Str
                            ) =>
                        {
                            'o'
                        }
                        _ => return None,
                    };
                    let mut value = value;
                    if operation == 'a' {
                        value.push("arrayhandle".into());
                    }
                    (
                        value,
                        operation,
                    )
                }
            };
            output.extend(value);
            if let thaw_hir::HirType::Optional(payload) | thaw_hir::HirType::Nullable(payload) = ty {
                let payload_offset = offset
                    + if matches!(payload.as_ref(), thaw_hir::HirType::Bool) {
                        1
                    } else {
                        8
                    };
                output.push(format!("objset{operation}{payload_offset}"));
                output.push(format!("c{:016x}", 1.0f64.to_bits()));
                output.push(format!("objsetb{offset}"));
            } else if let thaw_hir::HirType::Nullish(payload) = ty {
                let payload_offset = offset
                    + if matches!(payload.as_ref(), thaw_hir::HirType::Bool) {
                        1
                    } else {
                        8
                    };
                output.push(format!("objset{operation}{payload_offset}"));
            } else {
                output.push(format!("objset{operation}{offset}"));
            }
            offset += field_size(ty);
        }
        Some(output)
    }

    fn encode_fixed_object_union_return(
        object: &thaw_parser::ast::ObjectLit,
        fields: &[(String, thaw_hir::HirType)],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        let mut output =
            encode_fixed_object_value(object, fields, parameters, locals, context)?;
        output.push("tagobject".into());
        Some(JitExport::Value(validated_jit_expression(
            output,
            JitKind::Dynamic,
        )?))
    }

    fn encode_dictionary_return(
        object: &thaw_parser::ast::ObjectLit,
        element: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        object
            .props
            .iter()
            .map(|property| {
                let PropOrSpread::Prop(property) = property else {
                    let PropOrSpread::Spread(spread) = property else {
                        unreachable!()
                    };
                    return encode_return_expression(
                        spread.expr.as_ref(),
                        &thaw_hir::HirType::Dictionary(Box::new(element.clone())),
                        parameters,
                        locals,
                        context,
                    )
                    .map(JitDictionaryEntry::Spread);
                };
                let (key, value) = match property.as_ref() {
                    Prop::KeyValue(property) => {
                        let value = encode_return_expression(
                            property.value.as_ref(),
                            element,
                            parameters,
                            locals,
                            context,
                        )?;
                        let key = match &property.key {
                            PropName::Ident(identifier) => {
                                return Some(JitDictionaryEntry::Static(
                                    identifier.sym.to_string(),
                                    value,
                                ));
                            }
                            PropName::Str(string) => {
                                return Some(JitDictionaryEntry::Static(
                                    string.value.to_string_lossy().into_owned(),
                                    value,
                                ));
                            }
                            PropName::Num(number) => {
                                return Some(JitDictionaryEntry::Static(
                                    number.value.to_string(),
                                    value,
                                ));
                            }
                            PropName::Computed(computed) => encode_return_expression(
                                computed.expr.as_ref(),
                                &thaw_hir::HirType::Str,
                                parameters,
                                locals,
                                context,
                            )?,
                            _ => return None,
                        };
                        (key, value)
                    }
                    Prop::Shorthand(identifier) => {
                        let encoded = if let Some(value) = locals.get(identifier.sym.as_ref()) {
                            value.clone()
                        } else {
                            vec![parameters.get(identifier.sym.as_ref())?.clone()]
                        };
                        return Some(JitDictionaryEntry::Static(
                            identifier.sym.to_string(),
                            JitExport::Value(validated_jit_expression(
                            encoded,
                            jit_return_kind(element)?,
                            )?),
                        ));
                    }
                    _ => return None,
                };
                Some(JitDictionaryEntry::Computed(key, value))
            })
            .collect::<Option<Vec<_>>>()
            .map(JitExport::Dictionary)
    }

    fn jit_return_kind(ty: &thaw_hir::HirType) -> Option<JitKind> {
        match ty {
            thaw_hir::HirType::F64 => Some(JitKind::Number),
            thaw_hir::HirType::Bool => Some(JitKind::Boolean),
            thaw_hir::HirType::Str => Some(JitKind::String),
            thaw_hir::HirType::Union(elements)
                if jit_tagged_union(elements) =>
            {
                Some(JitKind::Dynamic)
            }
            thaw_hir::HirType::Array(element)
                if jit_array_result_element_supported(element) =>
            {
                Some(JitKind::Array)
            }
            thaw_hir::HirType::Dictionary(element)
                if matches!(
                    element.as_ref(),
                    thaw_hir::HirType::F64
                        | thaw_hir::HirType::Bool
                        | thaw_hir::HirType::Str
                ) =>
            {
                Some(JitKind::Dictionary)
            }
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload) => jit_return_kind(payload),
            _ => None,
        }
    }

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
                token.starts_with('r') || jit_dynamic_array_argument(token)
            }),
            _ => false,
        };
        let local_array = member_path(member.obj.as_ref())
            .and_then(|path| locals.get(&path))
            .and_then(|tokens| jit_expression_kind(tokens))
            .is_some_and(|(kind, _)| kind == JitKind::Array);
        if !parameter_array && !local_array {
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

    fn encode_expression(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        struct OptionalChainOperation {
            receiver_presence: Option<Vec<String>>,
            receiver: Vec<String>,
            continuation: Vec<String>,
        }

        fn optional_tokens<'a>(
            expression: &Expr,
            parameters: &'a std::collections::HashMap<String, String>,
        ) -> Option<(Vec<String>, &'a str)> {
            let expression = match expression {
                Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
                expression => expression,
            };
            let Expr::Ident(identifier) = expression else {
                return None;
            };
            let (presence, value) = parameters
                .get(identifier.sym.as_ref())?
                .strip_prefix("optional:")?
                .split_once(':')?;
            Some((presence.split(',').map(str::to_owned).collect(), value))
        }

        fn flatten_nullish<'a>(expression: &'a Expr, output: &mut Vec<&'a Expr>) {
            if let Expr::Bin(binary) = expression {
                if binary.op == BinaryOp::NullishCoalescing {
                    flatten_nullish(binary.left.as_ref(), output);
                    flatten_nullish(binary.right.as_ref(), output);
                    return;
                }
            }
            output.push(expression);
        }

        fn optional_chain_operation(
            chain: &thaw_parser::ast::OptChainExpr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<OptionalChainOperation> {
            const RECEIVER: &str = "__thaw_optional_receiver";

            fn ordinary_expression(expression: &Expr) -> Option<Expr> {
                match expression {
                    Expr::OptChain(chain) => ordinary_chain(chain),
                    Expr::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(ordinary_expression(member.obj.as_ref())?);
                        Some(Expr::Member(member))
                    }
                    Expr::Call(call) => {
                        let mut call = call.clone();
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = ordinary_expression(callee.as_ref())?;
                        }
                        Some(Expr::Call(call))
                    }
                    Expr::Paren(parenthesized) => {
                        let mut parenthesized = parenthesized.clone();
                        parenthesized.expr =
                            Box::new(ordinary_expression(parenthesized.expr.as_ref())?);
                        Some(Expr::Paren(parenthesized))
                    }
                    expression => Some(expression.clone()),
                }
            }

            fn split_expression(
                expression: &Expr,
                receiver: &mut Option<Expr>,
            ) -> Option<Expr> {
                match expression {
                    Expr::OptChain(chain) => split_chain(chain, receiver),
                    Expr::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(split_expression(member.obj.as_ref(), receiver)?);
                        Some(Expr::Member(member))
                    }
                    Expr::Call(call) => {
                        let mut call = call.clone();
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = split_expression(callee.as_ref(), receiver)?;
                        }
                        Some(Expr::Call(call))
                    }
                    Expr::Paren(parenthesized) => {
                        let mut parenthesized = parenthesized.clone();
                        parenthesized.expr =
                            Box::new(split_expression(parenthesized.expr.as_ref(), receiver)?);
                        Some(Expr::Paren(parenthesized))
                    }
                    expression => Some(expression.clone()),
                }
            }

            fn split_chain(
                chain: &thaw_parser::ast::OptChainExpr,
                receiver: &mut Option<Expr>,
            ) -> Option<Expr> {
                match chain.base.as_ref() {
                    OptChainBase::Member(member) => {
                        let mut member = member.clone();
                        if chain.optional && receiver.is_none() {
                            *receiver = Some(ordinary_expression(member.obj.as_ref())?);
                            member.obj = Box::new(Expr::Ident(RECEIVER.into()));
                        } else {
                            member.obj = Box::new(split_expression(member.obj.as_ref(), receiver)?);
                        }
                        Some(Expr::Member(member))
                    }
                    OptChainBase::Call(call) => {
                        if chain.optional {
                            return None;
                        }
                        let mut call = CallExpr::from(call.clone());
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = split_expression(callee.as_ref(), receiver)?;
                        }
                        Some(Expr::Call(call))
                    }
                }
            }

            fn ordinary_chain(chain: &thaw_parser::ast::OptChainExpr) -> Option<Expr> {
                match chain.base.as_ref() {
                    OptChainBase::Member(member) => {
                        let mut member = member.clone();
                        member.obj = Box::new(ordinary_expression(member.obj.as_ref())?);
                        Some(Expr::Member(member))
                    }
                    OptChainBase::Call(call) => {
                        let mut call = CallExpr::from(call.clone());
                        if let Callee::Expr(callee) = &mut call.callee {
                            **callee = ordinary_expression(callee.as_ref())?;
                        }
                        Some(Expr::Call(call))
                    }
                }
            }

            fn optional_root<'a>(
                expression: &Expr,
                parameters: &'a std::collections::HashMap<String, String>,
            ) -> Option<(Vec<String>, &'a str, String)> {
                match expression {
                    Expr::Ident(identifier) => {
                        let (presence, value) = parameters
                            .get(identifier.sym.as_ref())?
                            .strip_prefix("optional:")?
                            .split_once(':')?;
                        Some((
                            presence.split(',').map(str::to_owned).collect(),
                            value,
                            identifier.sym.to_string(),
                        ))
                    }
                    Expr::Member(member) => optional_root(member.obj.as_ref(), parameters),
                    Expr::Call(call) => match &call.callee {
                        Callee::Expr(callee) => optional_root(callee.as_ref(), parameters),
                        _ => None,
                    },
                    Expr::Paren(parenthesized) => {
                        optional_root(parenthesized.expr.as_ref(), parameters)
                    }
                    _ => None,
                }
            }

            let ordinary = ordinary_chain(chain)?;
            if let Some((presence, value, receiver)) = optional_root(&ordinary, parameters) {
                let mut unwrapped = parameters.clone();
                unwrapped.insert(receiver, value.into());
                let mut operation = Vec::new();
                encode_expression(
                    &ordinary,
                    &unwrapped,
                    locals,
                    context,
                    &mut operation,
                )?;
                return Some(OptionalChainOperation {
                    receiver_presence: Some(presence),
                    receiver: operation,
                    continuation: Vec::new(),
                });
            }

            let mut source = None;
            let ordinary = split_chain(chain, &mut source)?;
            let source = source?;
            let mut receiver = Vec::new();
            encode_expression(
                &source,
                parameters,
                locals,
                context,
                &mut receiver,
            )?;
            if !jit_operation_may_be_absent(&receiver) {
                return None;
            }
            let receiver_token = match jit_expression_kind(&receiver)?.0 {
                JitKind::Number => format!("a{RECEIVER}"),
                JitKind::Boolean => format!("b{RECEIVER}"),
                JitKind::String => format!("s{RECEIVER}"),
                JitKind::Dynamic => return None,
                JitKind::Array => format!("{}{}", array_prefix(&receiver)?, RECEIVER),
                JitKind::Dictionary => {
                    format!("{}{}", dictionary_prefix(&receiver)?, RECEIVER)
                }
            };
            let mut continuation_parameters = parameters.clone();
            continuation_parameters.insert(RECEIVER.into(), receiver_token.clone());
            let mut continuation_locals = locals.clone();
            if let Some(source_path) = member_path(&source) {
                for (path, operation) in locals {
                    let Some(suffix) = path.strip_prefix(&format!("{source_path}.")) else {
                        continue;
                    };
                    let Some(operation_suffix) = operation.strip_prefix(receiver.as_slice()) else {
                        continue;
                    };
                    let mut translated = vec![receiver_token.clone()];
                    translated.extend_from_slice(operation_suffix);
                    continuation_locals.insert(format!("{RECEIVER}.{suffix}"), translated);
                }
            }
            let mut continuation = Vec::new();
            encode_expression(
                &ordinary,
                &continuation_parameters,
                &continuation_locals,
                context,
                &mut continuation,
            )?;
            if continuation.first()? != &receiver_token {
                return None;
            }
            continuation.remove(0);
            Some(OptionalChainOperation {
                receiver_presence: None,
                receiver,
                continuation,
            })
        }

        fn encode_fixed_computed_field(
            path: &str,
            key: &Expr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_computed_field(
                    path,
                    parenthesized.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                ),
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))
                }
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_computed_field(
                        path,
                        conditional.cons.as_ref(),
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_computed_field(
                        path,
                        conditional.alt.as_ref(),
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                _ => None,
            }
        }

        fn encode_runtime_computed_field(
            path: &str,
            key: &Expr,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            fn branches(
                fields: &[(String, Vec<String>)],
                kind: JitKind,
                output: &mut Vec<String>,
            ) -> Option<()> {
                let Some(((name, value), remaining)) = fields.split_first() else {
                    output.push(
                        match kind {
                            JitKind::Number => "absentn",
                            JitKind::Boolean => "absentb",
                            JitKind::String => "absents",
                            JitKind::Array => "absenta",
                            JitKind::Dictionary => "absentd",
                            JitKind::Dynamic => "absentdyn",
                        }
                        .into(),
                    );
                    return Some(());
                };
                output.push("dup".into());
                encode_string(name, output)?;
                output.extend([
                    "strcmp".into(),
                    "c0000000000000000".into(),
                    "==".into(),
                    "if".into(),
                ]);
                output.extend(value.iter().cloned());
                output.push("else".into());
                branches(remaining, kind, output)?;
                output.push("end".into());
                Some(())
            }

            let prefix = format!("{path}.");
            let mut fields = locals
                .iter()
                .filter_map(|(name, value)| {
                    let field = name.strip_prefix(&prefix)?;
                    (!field.contains('.')).then(|| (field.to_owned(), value.clone()))
                })
                .chain(parameters.iter().filter_map(|(name, value)| {
                    let field = name.strip_prefix(&prefix)?;
                    (!field.contains('.')).then(|| (field.to_owned(), vec![value.clone()]))
                }))
                .collect::<Vec<_>>();
            fields.sort_by(|left, right| left.0.cmp(&right.0));
            fields.dedup_by(|left, right| left.0 == right.0);
            let kind = fields
                .iter()
                .map(|(_, value)| jit_expression_kind(value).map(|result| result.0))
                .collect::<Option<Vec<_>>>()?;
            let expected = *kind.first()?;
            if kind.iter().any(|kind| *kind != expected) {
                return None;
            }
            let mut output = Vec::new();
            encode_expression(key, parameters, locals, context, &mut output)?;
            if jit_expression_kind(&output)?.0 != JitKind::String {
                return None;
            }
            branches(&fields, expected, &mut output)?;
            output.push("nip".into());
            Some(output)
        }

        fn encode_fixed_field_assignment(
            path: &str,
            key: &Expr,
            value: &Expr,
            assignment: AssignOp,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_field_assignment(
                    path,
                    parenthesized.expr.as_ref(),
                    value,
                    assignment,
                    parameters,
                    locals,
                    context,
                ),
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_field_assignment(
                        path,
                        conditional.cons.as_ref(),
                        value,
                        assignment,
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_field_assignment(
                        path,
                        conditional.alt.as_ref(),
                        value,
                        assignment,
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    let operation = locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))?;
                    let (getter, receiver) = operation.split_last()?;
                    if matches!(assignment, AssignOp::AndAssign | AssignOp::OrAssign) {
                        let kind = jit_expression_kind(&operation)?.0;
                        let tagged = getter.starts_with("objopt")
                            || getter.starts_with("objnullable")
                            || getter.starts_with("objnull");
                        let preserve_absence = getter.starts_with("objnull")
                            && !getter.starts_with("objnullable");
                        if !matches!(kind, JitKind::Number | JitKind::Boolean | JitKind::String) {
                            return None;
                        }
                        let mut assigned = encode_fixed_field_assignment(
                            path,
                            key,
                            value,
                            AssignOp::Assign,
                            parameters,
                            locals,
                            context,
                        )?;
                        let mut current = operation;
                        normalize_callable_branches([&mut current, &mut assigned])?;
                        if tagged {
                            let absent = match (kind, preserve_absence) {
                                (JitKind::Number, false) => "absentn",
                                (JitKind::Boolean, false) => "absentb",
                                (JitKind::String, false) => "absents",
                                (JitKind::Number, true) => "keepabsentn",
                                (JitKind::Boolean, true) => "keepabsentb",
                                (JitKind::String, true) => "keepabsents",
                                _ => unreachable!(),
                            };
                            let fallback = assigned.clone();
                            current.push("ifpresent".into());
                            current.push("dup".into());
                            match kind {
                                JitKind::Number => current.push("asbool".into()),
                                JitKind::String => current.push("strbool".into()),
                                JitKind::Boolean => {}
                                _ => unreachable!(),
                            }
                            current.push(
                                if assignment == AssignOp::AndAssign {
                                    "&&"
                                } else {
                                    "||"
                                }
                                .into(),
                            );
                            current.extend(assigned);
                            current.push("end".into());
                            current.push("else".into());
                            if assignment == AssignOp::OrAssign {
                                current.extend(fallback);
                            } else {
                                current.push(absent.into());
                            }
                            current.push("end".into());
                            return Some(current);
                        }
                        current.push("dup".into());
                        match kind {
                            JitKind::Number => current.push("asbool".into()),
                            JitKind::String => current.push("strbool".into()),
                            JitKind::Boolean => {}
                            _ => unreachable!(),
                        }
                        current.push(
                            if assignment == AssignOp::AndAssign {
                                "&&"
                            } else {
                                "||"
                            }
                            .into(),
                        );
                        current.extend(assigned);
                        current.push("end".into());
                        return Some(current);
                    }
                    if assignment == AssignOp::NullishAssign {
                        let tagged = getter.starts_with("objopt")
                            || getter.starts_with("objnullable")
                            || getter.starts_with("objnull");
                        if !tagged {
                            return Some(operation);
                        }
                        let mut assigned = encode_fixed_field_assignment(
                            path,
                            key,
                            value,
                            AssignOp::Assign,
                            parameters,
                            locals,
                            context,
                        )?;
                        let mut present = operation;
                        normalize_callable_branches([&mut present, &mut assigned])?;
                        present.push("ifpresent".into());
                        present.push("else".into());
                        present.extend(assigned);
                        present.push("end".into());
                        return Some(present);
                    }
                    if assignment != AssignOp::Assign {
                        if let Some((semantic, offset)) = getter
                            .strip_prefix("objoptn")
                            .map(|offset| ('o', offset))
                            .or_else(|| {
                                getter
                                    .strip_prefix("objnullablen")
                                    .map(|offset| ('l', offset))
                            })
                            .or_else(|| {
                                getter.strip_prefix("objnulln").map(|offset| ('n', offset))
                            })
                        {
                            let operation = match assignment {
                                AssignOp::AddAssign => 'a',
                                AssignOp::SubAssign => 's',
                                AssignOp::MulAssign => 'm',
                                AssignOp::DivAssign => 'd',
                                AssignOp::ModAssign => 'r',
                                AssignOp::LShiftAssign => 'l',
                                AssignOp::RShiftAssign => 'h',
                                AssignOp::ZeroFillRShiftAssign => 'u',
                                AssignOp::BitOrAssign => 'o',
                                AssignOp::BitXorAssign => 'x',
                                AssignOp::BitAndAssign => 'b',
                                AssignOp::ExpAssign => 'p',
                                AssignOp::Assign
                                | AssignOp::AndAssign
                                | AssignOp::OrAssign
                                | AssignOp::NullishAssign => return None,
                            };
                            let offset = offset.parse::<u16>().ok()?;
                            let mut encoded = Vec::new();
                            encode_expression(
                                value,
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            let mut output = receiver.to_vec();
                            append_number(encoded, &mut output)?;
                            output.push(format!("objca{semantic}{operation}{offset}"));
                            return Some(output);
                        }
                    }
                    let (prefix, expected, offset, present_tag) = if getter
                        .strip_prefix("objn")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsetn",
                            JitKind::Number,
                            getter.strip_prefix("objn")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if getter
                        .strip_prefix("objb")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsetb",
                            JitKind::Boolean,
                            getter.strip_prefix("objb")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if getter
                        .strip_prefix("objs")
                        .is_some_and(|offset| offset.parse::<u16>().is_ok())
                    {
                        (
                            "objsets",
                            JitKind::String,
                            getter.strip_prefix("objs")?.parse::<u16>().ok()?,
                            None,
                        )
                    } else if assignment == AssignOp::Assign {
                        let (encoded, present_tag) = getter
                            .strip_prefix("objopt")
                            .map(|offset| (offset, 1u8))
                            .or_else(|| {
                                getter
                                    .strip_prefix("objnullable")
                                    .map(|offset| (offset, 1u8))
                            })
                            .or_else(|| getter.strip_prefix("objnull").map(|offset| (offset, 0u8)))?;
                        let (kind, offset) = encoded.split_at(1);
                        let offset = offset.parse::<u16>().ok()?;
                        let (prefix, expected, payload_offset) = match kind {
                            "n" => ("objsetn", JitKind::Number, offset.checked_add(8)?),
                            "b" => ("objsetb", JitKind::Boolean, offset.checked_add(1)?),
                            "s" => ("objsets", JitKind::String, offset.checked_add(8)?),
                            _ => return None,
                        };
                        (prefix, expected, payload_offset, Some((offset, present_tag)))
                    } else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(value, parameters, locals, context, &mut encoded)?;
                    let mut output = receiver.to_vec();
                    if assignment == AssignOp::Assign {
                        if !jit_kind_compatible(jit_expression_kind(&encoded)?.0, expected) {
                            return None;
                        }
                        output.extend(encoded);
                    } else if assignment == AssignOp::AddAssign && expected == JitKind::String {
                        output.push("dup".into());
                        output.push(getter.clone());
                        append_string(encoded, &mut output)?;
                        output.push("concat".into());
                    } else {
                        if expected != JitKind::Number
                            || jit_expression_kind(&encoded)?.0 == JitKind::Array
                        {
                            return None;
                        }
                        output.push("dup".into());
                        output.push(getter.clone());
                        append_number(encoded, &mut output)?;
                        output.push(
                            match assignment {
                                AssignOp::AddAssign => "+",
                                AssignOp::SubAssign => "-",
                                AssignOp::MulAssign => "*",
                                AssignOp::DivAssign => "/",
                                AssignOp::ModAssign => "%",
                                AssignOp::LShiftAssign => "shl",
                                AssignOp::RShiftAssign => "shr",
                                AssignOp::ZeroFillRShiftAssign => "ushr",
                                AssignOp::BitOrAssign => "bor",
                                AssignOp::BitXorAssign => "bxor",
                                AssignOp::BitAndAssign => "band",
                                AssignOp::ExpAssign => "pow",
                                AssignOp::Assign
                                | AssignOp::AndAssign
                                | AssignOp::OrAssign
                                | AssignOp::NullishAssign => return None,
                            }
                            .into(),
                        );
                    }
                    output.extend([
                        "dup2".into(),
                        format!("{prefix}{offset}"),
                    ]);
                    if let Some((tag_offset, tag)) = present_tag {
                        output.push(format!("c{:016x}", f64::from(tag).to_bits()));
                        output.push(format!("objsetb{tag_offset}"));
                    }
                    output.extend(["drop".into(), "nip".into()]);
                    Some(output)
                }
                _ => None,
            }
        }

        fn encode_fixed_field_update(
            path: &str,
            key: &Expr,
            update: UpdateOp,
            prefix: bool,
            parameters: &std::collections::HashMap<String, String>,
            locals: &std::collections::HashMap<String, Vec<String>>,
            context: &mut InlineContext<'_>,
        ) -> Option<Vec<String>> {
            match key {
                Expr::Paren(parenthesized) => encode_fixed_field_update(
                    path,
                    parenthesized.expr.as_ref(),
                    update,
                    prefix,
                    parameters,
                    locals,
                    context,
                ),
                Expr::Cond(conditional) => {
                    let mut output = Vec::new();
                    encode_condition(
                        conditional.test.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut output,
                    )?;
                    let mut consequent = encode_fixed_field_update(
                        path,
                        conditional.cons.as_ref(),
                        update,
                        prefix,
                        parameters,
                        locals,
                        context,
                    )?;
                    let mut alternate = encode_fixed_field_update(
                        path,
                        conditional.alt.as_ref(),
                        update,
                        prefix,
                        parameters,
                        locals,
                        context,
                    )?;
                    normalize_callable_branches([&mut consequent, &mut alternate])?;
                    output.push("if".into());
                    output.extend(consequent);
                    output.push("else".into());
                    output.extend(alternate);
                    output.push("end".into());
                    Some(output)
                }
                Expr::Lit(Lit::Str(property)) => {
                    let field = format!("{path}.{}", property.value.to_string_lossy());
                    let operation = locals
                        .get(&field)
                        .cloned()
                        .or_else(|| parameters.get(&field).map(|value| vec![value.clone()]))?;
                    let (getter, receiver) = operation.split_last()?;
                    if let Some((semantic, offset)) = getter
                        .strip_prefix("objoptn")
                        .map(|offset| ('o', offset))
                        .or_else(|| {
                            getter
                                .strip_prefix("objnullablen")
                                .map(|offset| ('l', offset))
                        })
                        .or_else(|| getter.strip_prefix("objnulln").map(|offset| ('n', offset)))
                    {
                        let offset = offset.parse::<u16>().ok()?;
                        let operation = match update {
                            UpdateOp::PlusPlus => 'i',
                            UpdateOp::MinusMinus => 'd',
                        };
                        let result = if prefix { 'p' } else { 'o' };
                        let mut output = receiver.to_vec();
                        output.push(format!("objup{semantic}{operation}{result}{offset}"));
                        return Some(output);
                    }
                    let offset = getter.strip_prefix("objn")?.parse::<u16>().ok()?;
                    let mut output = receiver.to_vec();
                    output.extend(["dup".into(), getter.clone()]);
                    if !prefix {
                        output.push("dup2".into());
                    }
                    output.push(format!("c{:016x}", 1.0f64.to_bits()));
                    output.push(
                        match update {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    if prefix {
                        output.push("dup2".into());
                    }
                    output.extend([
                        format!("objsetn{offset}"),
                        "drop".into(),
                        "nip".into(),
                    ]);
                    Some(output)
                }
                _ => None,
            }
        }

        match expression {
            Expr::Ident(identifier) if locals.contains_key(identifier.sym.as_ref()) => {
                output.extend(locals.get(identifier.sym.as_ref())?.iter().cloned());
            }
            Expr::Ident(identifier) if parameters.contains_key(identifier.sym.as_ref()) => {
                let value = parameters.get(identifier.sym.as_ref())?;
                if value.starts_with("optional:") {
                    return None;
                }
                output.push(value.clone());
            }
            Expr::Ident(_) => output.push(format!(
                "c{:016x}",
                numeric_constant(expression, parameters, locals, context.helpers)?.to_bits()
            )),
            Expr::Lit(Lit::Num(number)) => {
                output.push(format!("c{:016x}", number.value.to_bits()));
            }
            Expr::Lit(Lit::Bool(boolean)) => {
                output.push(format!(
                    "c{:016x}",
                    f64::from(u8::from(boolean.value)).to_bits()
                ));
            }
            Expr::Lit(Lit::Str(string)) => {
                let string = string.value.to_string_lossy();
                encode_string(&string, output)?;
            }
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut emitted = false;
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let value = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy())
                        .unwrap_or_else(|| quasi.raw.to_string().into());
                    if !value.is_empty() {
                        encode_string(&value, output)?;
                        if emitted {
                            output.push("concat".into());
                        }
                        emitted = true;
                    }
                    if let Some(expression) = template.exprs.get(index) {
                        let mut encoded = Vec::new();
                        encode_expression(expression, parameters, locals, context, &mut encoded)?;
                        append_string(encoded, output)?;
                        if emitted {
                            output.push("concat".into());
                        }
                        emitted = true;
                    }
                }
                if !emitted {
                    output.push("t".into());
                }
            }
            Expr::Array(array) => {
                output.push("arrayempty".into());
                let mut prefix = None;
                for element in &array.elems {
                    let element = element.as_ref()?;
                    append_array_element(
                        element,
                        parameters,
                        locals,
                        context,
                        &mut prefix,
                        output,
                    )?;
                }
            }
            Expr::Object(object) => {
                let mut entries = Vec::new();
                let mut kind = None;
                for property in &object.props {
                    let (key, value) = object_property(property)?;
                    let mut encoded = Vec::new();
                    match value {
                        ObjectReturnValue::Expression(expression) => encode_expression(
                            expression,
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?,
                        ObjectReturnValue::Shorthand(identifier) => encode_expression(
                            &Expr::Ident(identifier.clone()),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?,
                    }
                    let value_kind = jit_expression_kind(&encoded)?.0;
                    if matches!(value_kind, JitKind::Array | JitKind::Dictionary)
                        || kind.replace(value_kind).is_some_and(|kind| kind != value_kind)
                    {
                        return None;
                    }
                    entries.push((key, encoded));
                }
                let prefix = match kind? {
                    JitKind::Number => "dn",
                    JitKind::Boolean => "db",
                    JitKind::String => "ds",
                    JitKind::Dynamic | JitKind::Array | JitKind::Dictionary => return None,
                };
                output.push(format!("{prefix}empty"));
                for (key, value) in entries {
                    let mut encoded_key = Vec::new();
                    encode_string(&key, &mut encoded_key)?;
                    let encoded_key = encoded_key.pop()?.strip_prefix('t')?.to_owned();
                    output.extend(value);
                    output.push(format!("{prefix}put{encoded_key}"));
                }
            }
            Expr::Member(member) => {
                let common_array_receiver = matches!(&member.prop, MemberProp::Computed(_))
                    && matches!(member.obj.as_ref(), Expr::Ident(identifier)
                        if locals.get(identifier.sym.as_ref()).is_some_and(|value| {
                            matches!(value.last().map(String::as_str), Some("untagarrayn" | "untagarrayb" | "untagarrays"))
                        }));
                let mut object_field = (!common_array_receiver)
                    .then(|| member_path(expression))
                    .flatten()
                    .and_then(|path| {
                        locals
                            .get(&path)
                            .cloned()
                            .or_else(|| parameters.get(&path).map(|field| vec![field.clone()]))
                    });
                if object_field.is_none() {
                    if let MemberProp::Computed(computed) = &member.prop {
                        if let Some(path) = member_path(member.obj.as_ref()) {
                            object_field = encode_fixed_computed_field(
                                &path,
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                            )
                            .or_else(|| {
                                encode_runtime_computed_field(
                                    &path,
                                    computed.expr.as_ref(),
                                    parameters,
                                    locals,
                                    context,
                                )
                            });
                        }
                    }
                }
                if matches!(
                    (member.obj.as_ref(), &member.prop),
                    (Expr::Ident(object), MemberProp::Ident(property))
                        if object.sym == "process"
                            && matches!(property.sym.as_ref(), "pid" | "ppid")
                            && !parameters.contains_key("process")
                            && !locals.contains_key("process")
                ) {
                    let MemberProp::Ident(property) = &member.prop else {
                        unreachable!();
                    };
                    output.push(format!("process{}", property.sym));
                } else if let Some(field) = object_field {
                    output.extend(field);
                } else if let MemberProp::Computed(computed) = &member.prop {
                    let mut receiver = Vec::new();
                    encode_expression(
                        member.obj.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut receiver,
                    )?;
                    match jit_expression_kind(&receiver)?.0 {
                        JitKind::Array => {
                            let prefix = array_prefix(&receiver)?;
                            output.extend(receiver);
                            encode_number(
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            output.push(format!("{prefix}get"));
                        }
                        JitKind::Dictionary => {
                            let prefix = dictionary_prefix(&receiver)?;
                            let mut key = Vec::new();
                            encode_expression(
                                computed.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut key,
                            )?;
                            if jit_expression_kind(&key)?.0 != JitKind::String {
                                return None;
                            }
                            output.extend(receiver);
                            output.extend(key);
                            output.push(format!("{prefix}get"));
                        }
                        _ => return None,
                    }
                } else if let MemberProp::Ident(property) = &member.prop {
                    let mut receiver = Vec::new();
                    let encoded_receiver = encode_expression(
                        member.obj.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut receiver,
                    )
                    .is_some();
                    if receiver.len() == 1 && jit_dynamic_array_argument(&receiver[0]) {
                        receiver.push("untagarray".into());
                    }
                    if encoded_receiver
                        && jit_expression_kind(&receiver)?.0 == JitKind::Dictionary
                    {
                        let prefix = dictionary_prefix(&receiver)?;
                        output.extend(receiver);
                        encode_string(property.sym.as_ref(), output)?;
                        output.push(format!("{prefix}get"));
                    } else if property.sym == "length" {
                        let operation = match jit_expression_kind(&receiver)?.0 {
                            JitKind::String => "strlen",
                            JitKind::Array => "arraylen",
                            _ => return None,
                        };
                        output.extend(receiver);
                        output.push(operation.into());
                    } else {
                        output.push(format!(
                            "c{:016x}",
                            numeric_constant(expression, parameters, locals, context.helpers)?
                                .to_bits()
                        ));
                    }
                } else {
                    output.push(format!(
                        "c{:016x}",
                        numeric_constant(expression, parameters, locals, context.helpers)?.to_bits()
                    ));
                }
            }
            Expr::Assign(assignment) => {
                let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left
                else {
                    return None;
                };
                let key = match &target.prop {
                    MemberProp::Ident(property) => {
                        Expr::Lit(Lit::Str(property.sym.to_string().into()))
                    }
                    MemberProp::Computed(property) => property.expr.as_ref().clone(),
                    MemberProp::PrivateName(_) => return None,
                };
                if let Some(path) = member_path(target.obj.as_ref()) {
                    if let Some(encoded) = encode_fixed_field_assignment(
                        &path,
                        &key,
                        assignment.right.as_ref(),
                        assignment.op,
                        parameters,
                        locals,
                        context,
                    ) {
                        output.extend(encoded);
                        return (output.len() <= 256).then_some(());
                    }
                }
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                let dynamic_array = receiver.last().is_some_and(|token| token == "untagarray");
                if dynamic_array {
                    receiver.pop();
                }
                let receiver_kind = if dynamic_array {
                    JitKind::Dynamic
                } else {
                    jit_expression_kind(&receiver)?.0
                };
                let prefix = match receiver_kind {
                    JitKind::Array => array_prefix(&receiver)?,
                    JitKind::Dictionary => dictionary_prefix(&receiver)?,
                    JitKind::Dynamic if dynamic_array => "dynamic",
                    _ => return None,
                };
                let expected = match prefix.as_bytes().get(1) {
                    Some(b'n') => JitKind::Number,
                    Some(b's') => JitKind::String,
                    Some(b'b') => JitKind::Boolean,
                    _ if dynamic_array => JitKind::Dynamic,
                    _ => return None,
                };
                let local_set = if assignment.op == AssignOp::Assign {
                    match target.obj.as_ref() {
                        Expr::Ident(identifier) => locals
                            .get(identifier.sym.as_ref())
                            .and_then(|tokens| tokens.first())
                            .and_then(|local| {
                                local
                                    .starts_with(&format!("{prefix}l"))
                                    .then(|| loop_local_index(local))
                                    .flatten()
                            })
                            .map(|index| format!("{prefix}lset{index}")),
                        _ => None,
                    }
                } else {
                    None
                };
                if local_set.is_none() {
                    output.extend(receiver);
                }
                match (&target.prop, receiver_kind) {
                    (MemberProp::Computed(index), JitKind::Array | JitKind::Dynamic) => {
                        encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    }
                    (MemberProp::Computed(key), JitKind::Dictionary) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    (MemberProp::Ident(key), JitKind::Dictionary) => {
                        encode_string(key.sym.as_ref(), output)?;
                    }
                    _ => return None,
                }
                if assignment.op != AssignOp::Assign {
                    output.push("dup2".into());
                    output.push(format!("{prefix}get"));
                }
                let mut value = Vec::new();
                encode_expression(assignment.right.as_ref(), parameters, locals, context, &mut value)?;
                if assignment.op == AssignOp::Assign {
                    if jit_expression_kind(&value)?.0 != expected {
                        return None;
                    }
                    output.extend(value);
                } else if assignment.op == AssignOp::AddAssign && expected == JitKind::String {
                    append_string(value, output)?;
                    output.push("concat".into());
                } else {
                    if expected != JitKind::Number || jit_expression_kind(&value)?.0 == JitKind::Array {
                        return None;
                    }
                    append_number(value, output)?;
                    if assignment.op != AssignOp::Assign {
                        output.push(
                            match assignment.op {
                                AssignOp::AddAssign => "+",
                                AssignOp::SubAssign => "-",
                                AssignOp::MulAssign => "*",
                                AssignOp::DivAssign => "/",
                                AssignOp::ModAssign => "%",
                                AssignOp::LShiftAssign => "shl",
                                AssignOp::RShiftAssign => "shr",
                                AssignOp::ZeroFillRShiftAssign => "ushr",
                                AssignOp::BitOrAssign => "bor",
                                AssignOp::BitXorAssign => "bxor",
                                AssignOp::BitAndAssign => "band",
                                AssignOp::ExpAssign => "pow",
                                _ => return None,
                            }
                            .into(),
                        );
                    }
                }
                output.push(if dynamic_array {
                    "dynarrayset".into()
                } else {
                    local_set.unwrap_or_else(|| format!("{prefix}set"))
                });
            }
            Expr::Update(update) => {
                let Expr::Member(target) = update.arg.as_ref() else {
                    return None;
                };
                let key = match &target.prop {
                    MemberProp::Ident(property) => {
                        Expr::Lit(Lit::Str(property.sym.to_string().into()))
                    }
                    MemberProp::Computed(property) => property.expr.as_ref().clone(),
                    MemberProp::PrivateName(_) => return None,
                };
                if let Some(path) = member_path(target.obj.as_ref()) {
                    if let Some(encoded) = encode_fixed_field_update(
                        &path,
                        &key,
                        update.op,
                        update.prefix,
                        parameters,
                        locals,
                        context,
                    ) {
                        output.extend(encoded);
                        return (output.len() <= 256).then_some(());
                    }
                }
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                let receiver_kind = jit_expression_kind(&receiver)?.0;
                let prefix = match receiver_kind {
                    JitKind::Array if array_prefix(&receiver)? == "rn" => "rn",
                    JitKind::Dictionary if dictionary_prefix(&receiver)? == "dn" => "dn",
                    _ => return None,
                };
                output.extend(receiver);
                match (&target.prop, receiver_kind) {
                    (MemberProp::Computed(index), JitKind::Array) => {
                        encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    }
                    (MemberProp::Computed(key), JitKind::Dictionary) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    (MemberProp::Ident(key), JitKind::Dictionary) => {
                        encode_string(key.sym.as_ref(), output)?;
                    }
                    _ => return None,
                }
                if prefix != "rn" && prefix != "dn" {
                    return None;
                }
                output.push("dup2".into());
                output.push(format!("{prefix}get"));
                if !update.prefix {
                    output.push("dup".into());
                }
                output.push(format!("c{:016x}", 1.0f64.to_bits()));
                output.push(
                    match update.op {
                        UpdateOp::PlusPlus => "+",
                        UpdateOp::MinusMinus => "-",
                    }
                    .into(),
                );
                output.push(
                    if update.prefix {
                        format!("{prefix}set")
                    } else {
                        format!("{prefix}postset")
                    },
                );
            }
            Expr::Unary(unary) if unary.op == UnaryOp::Delete => {
                let Expr::Member(target) = unary.arg.as_ref() else {
                    return None;
                };
                let mut receiver = Vec::new();
                encode_expression(
                    target.obj.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut receiver,
                )?;
                if jit_expression_kind(&receiver)?.0 != JitKind::Dictionary {
                    return None;
                }
                output.extend(receiver);
                match &target.prop {
                    MemberProp::Computed(key) => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            key.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        if jit_expression_kind(&encoded)?.0 != JitKind::String {
                            return None;
                        }
                        output.extend(encoded);
                    }
                    MemberProp::Ident(key) => encode_string(key.sym.as_ref(), output)?,
                    _ => return None,
                }
                output.push("ddelete".into());
            }
            Expr::Unary(unary)
                if matches!(
                    unary.op,
                    UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Tilde | UnaryOp::Bang
                ) =>
            {
                if unary.op == UnaryOp::Bang {
                    let mut encoded = Vec::new();
                    encode_expression(unary.arg.as_ref(), parameters, locals, context, &mut encoded)?;
                    append_boolean(encoded, output)?;
                } else {
                    encode_number(unary.arg.as_ref(), parameters, locals, context, output)?;
                }
                match unary.op {
                    UnaryOp::Minus => output.push("neg".into()),
                    UnaryOp::Tilde => output.push("bnot".into()),
                    UnaryOp::Bang => output.push("boolnot".into()),
                    _ => {}
                }
            }
            Expr::Unary(unary) if unary.op == UnaryOp::TypeOf => {
                let mut encoded = Vec::new();
                encode_expression(unary.arg.as_ref(), parameters, locals, context, &mut encoded)?;
                let operation = match jit_expression_kind(&encoded)?.0 {
                    JitKind::Number => "typeofnumber",
                    JitKind::Boolean => "typeofboolean",
                    JitKind::String => "typeofstring",
                    JitKind::Dynamic => "typeofdynamic",
                    JitKind::Array | JitKind::Dictionary => "typeofobject",
                };
                output.extend(encoded);
                output.push(operation.into());
            }
            Expr::Paren(parenthesized) => {
                encode_expression(parenthesized.expr.as_ref(), parameters, locals, context, output)?;
            }
            Expr::Bin(binary) if binary.op == BinaryOp::Add => {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                append_add(left, right, output)?;
            }
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::LShift
                        | BinaryOp::RShift
                        | BinaryOp::ZeroFillRShift
                        | BinaryOp::Exp
                ) =>
            {
                for operand in [&binary.left, &binary.right] {
                    encode_number(operand.as_ref(), parameters, locals, context, output)?;
                }
                output.push(
                    match binary.op {
                        BinaryOp::Sub => "-",
                        BinaryOp::Mul => "*",
                        BinaryOp::Div => "/",
                        BinaryOp::Mod => "%",
                        BinaryOp::BitAnd => "band",
                        BinaryOp::BitOr => "bor",
                        BinaryOp::BitXor => "bxor",
                        BinaryOp::LShift => "shl",
                        BinaryOp::RShift => "shr",
                        BinaryOp::ZeroFillRShift => "ushr",
                        BinaryOp::Exp => "pow",
                        _ => unreachable!(),
                    }
                    .into(),
                );
            }
            Expr::Bin(binary)
                if matches!(binary.op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) =>
            {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                let left_kind = jit_expression_kind(&left)?.0;
                let right_kind = jit_expression_kind(&right)?.0;
                let kind = if left_kind == right_kind {
                    left_kind
                } else if left_kind == JitKind::Dynamic
                    && matches!(right_kind, JitKind::Number | JitKind::Boolean | JitKind::String)
                {
                    let mut tagged = Vec::new();
                    append_dynamic(right, &mut tagged)?;
                    right = tagged;
                    JitKind::Dynamic
                } else if right_kind == JitKind::Dynamic
                    && matches!(left_kind, JitKind::Number | JitKind::Boolean | JitKind::String)
                {
                    let mut tagged = Vec::new();
                    append_dynamic(left, &mut tagged)?;
                    left = tagged;
                    JitKind::Dynamic
                } else {
                    return None;
                };
                if matches!(kind, JitKind::Array | JitKind::Dictionary) {
                    return None;
                }
                output.extend(left);
                output.push("dup".into());
                match kind {
                    JitKind::Number => output.push("asbool".into()),
                    JitKind::String => output.push("strbool".into()),
                    JitKind::Boolean => {}
                    JitKind::Dynamic => output.push("dynbool".into()),
                    JitKind::Array | JitKind::Dictionary => unreachable!(),
                }
                output.push(if binary.op == BinaryOp::LogicalAnd {
                    "&&"
                } else {
                    "||"
                }
                .into());
                output.extend(right);
                output.push("end".into());
            }
            Expr::Bin(binary) if binary.op == BinaryOp::NullishCoalescing => {
                let mut operands = Vec::new();
                flatten_nullish(expression, &mut operands);
                let mut selected = Vec::new();
                encode_expression(
                    operands.pop()?,
                    parameters,
                    locals,
                    context,
                    &mut selected,
                )?;
                while let Some(operand) = operands.pop() {
                    let fallback = selected;
                    if let Some((presence, value)) = optional_tokens(operand, parameters) {
                        let mut present = vec![value.into()];
                        let mut fallback = fallback;
                        normalize_callable_branches([&mut present, &mut fallback])?;
                        selected = presence;
                        selected.extend(["asbool".into(), "if".into()]);
                        selected.extend(present);
                        selected.push("else".into());
                        selected.extend(fallback);
                        selected.push("end".into());
                    } else if let Expr::OptChain(chain) = operand {
                        let optional =
                            optional_chain_operation(chain, parameters, locals, context)?;
                        if let Some(presence) = optional.receiver_presence {
                            selected = presence;
                            selected.extend(["asbool".into(), "if".into()]);
                            selected.extend(optional.receiver.clone());
                            selected.extend(optional.continuation);
                            if jit_operation_may_be_absent(&optional.receiver) {
                                selected.push("ifpresent".into());
                                selected.push("else".into());
                                selected.extend(fallback.clone());
                                selected.push("end".into());
                            }
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        } else {
                            selected = optional.receiver;
                            selected.push("ifpresent".into());
                            selected.extend(optional.continuation.clone());
                            if jit_operation_may_be_absent(&optional.continuation) {
                                selected.push("ifpresent".into());
                                selected.push("else".into());
                                selected.extend(fallback.clone());
                                selected.push("end".into());
                            }
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        }
                    } else {
                        let mut operation = Vec::new();
                        encode_expression(
                            operand,
                            parameters,
                            locals,
                            context,
                            &mut operation,
                        )?;
                        if jit_operation_may_be_absent(&operation) {
                            selected = operation;
                            selected.push("ifpresent".into());
                            selected.push("else".into());
                            selected.extend(fallback);
                            selected.push("end".into());
                        } else {
                            selected = operation;
                        }
                    }
                }
                output.extend(selected);
            }
            Expr::OptChain(chain) => {
                let optional = optional_chain_operation(chain, parameters, locals, context)?;
                let mut operation = optional.receiver.clone();
                operation.extend(optional.continuation.clone());
                let kind = jit_expression_kind(&operation)?.0;
                if let Some(presence) = optional.receiver_presence {
                    output.extend(presence);
                    output.push("asbool".into());
                    output.push("if".into());
                    output.extend(operation);
                } else {
                    output.extend(optional.receiver);
                    output.push("ifpresent".into());
                    output.extend(optional.continuation);
                }
                output.push("else".into());
                output.push(
                    match kind {
                        JitKind::Number => "absentn",
                        JitKind::Boolean => "absentb",
                        JitKind::String => "absents",
                        JitKind::Dynamic => "absentdyn",
                        JitKind::Array => "absenta",
                        JitKind::Dictionary => "absentd",
                    }
                    .into(),
                );
                output.push("end".into());
            }
            Expr::Bin(binary) if binary.op == BinaryOp::In => {
                let mut key = Vec::new();
                encode_expression(
                    binary.left.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                append_string(key, output)?;
                let mut object = Vec::new();
                encode_expression(
                    binary.right.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut object,
                )?;
                if jit_expression_kind(&object)?.0 != JitKind::Dictionary {
                    return None;
                }
                output.extend(object);
                output.push("din".into());
            }
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::Lt
                        | BinaryOp::LtEq
                        | BinaryOp::Gt
                        | BinaryOp::GtEq
                        | BinaryOp::EqEq
                        | BinaryOp::EqEqEq
                        | BinaryOp::NotEq
                        | BinaryOp::NotEqEq
                ) =>
            {
                encode_condition(expression, parameters, locals, context, output)?;
            }
            Expr::Call(call) if number_format_method(call, parameters, locals).is_some() => {
                let (operation, receiver) = number_format_method(call, parameters, locals)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                let receiver_kind = jit_expression_kind(&encoded)?.0;
                if operation == "toradix" && call.args.is_empty() {
                    match receiver_kind {
                        JitKind::Boolean => {
                            output.extend(encoded);
                            output.push("boolstr".into());
                        }
                        JitKind::String => output.extend(encoded),
                        JitKind::Number => append_string(encoded, output)?,
                        JitKind::Dynamic => append_string(encoded, output)?,
                        JitKind::Array | JitKind::Dictionary => return None,
                    }
                    return Some(());
                }
                if receiver_kind != JitKind::Number {
                    return None;
                }
                match call.args.as_slice() {
                    [] if operation == "tofixed" => {
                        output.extend(encoded);
                        output.push(format!("c{:016x}", 0.0f64.to_bits()));
                        output.push(operation.into());
                    }
                    [] if operation == "toexponential" => {
                        output.extend(encoded);
                        output.push("toexponential0".into());
                    }
                    [] => append_string(encoded, output)?,
                    [argument] => {
                        output.extend(encoded);
                        encode_number(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                        output.push(operation.into());
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if object_assign_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let arguments = object_assign_call(call, parameters, locals, context.helpers)?;
                let mut target = Vec::new();
                encode_expression(
                    arguments.first()?.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut target,
                )?;
                if jit_expression_kind(&target)?.0 != JitKind::Dictionary {
                    return None;
                }
                let prefix = dictionary_prefix(&target)?;
                output.extend(target);
                for source in &arguments[1..] {
                    let mut encoded = Vec::new();
                    encode_expression(
                        source.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    if jit_expression_kind(&encoded)?.0 != JitKind::Dictionary
                        || dictionary_prefix(&encoded)? != prefix
                    {
                        return None;
                    }
                    output.extend(encoded);
                    output.push("dassign".into());
                }
            }
            Expr::Call(call)
                if object_from_entries_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let entries = object_from_entries_call(call, parameters, locals, context.helpers)?;
                let mut encoded = Vec::new();
                encode_expression(entries, parameters, locals, context, &mut encoded)?;
                if jit_expression_kind(&encoded)?.0 != JitKind::Array {
                    return None;
                }
                let operation = match entry_prefix(&encoded)? {
                    "dn" => "dnfromentries",
                    "db" => "dbfromentries",
                    "ds" => "dsfromentries",
                    _ => unreachable!(),
                };
                output.extend(encoded);
                output.push(operation.into());
            }
            Expr::Call(call)
                if object_dictionary_call(call, parameters, locals, context.helpers).is_some() =>
            {
                let (operation, object, key) =
                    object_dictionary_call(call, parameters, locals, context.helpers)?;
                let mut encoded = Vec::new();
                encode_expression(object, parameters, locals, context, &mut encoded)?;
                if jit_expression_kind(&encoded)?.0 != JitKind::Dictionary {
                    return None;
                }
                let operation = if matches!(operation, "dvalues" | "dentries") {
                    let entries = operation == "dentries";
                    match dictionary_prefix(&encoded)? {
                        "dn" if !entries => "dnvalues",
                        "db" if !entries => "dbvalues",
                        "ds" if !entries => "dsvalues",
                        "dn" => "dnentries",
                        "db" => "dbentries",
                        "ds" => "dsentries",
                        _ => unreachable!(),
                    }
                } else {
                    operation
                };
                output.extend(encoded);
                if let Some(key) = key {
                    let mut encoded = Vec::new();
                    encode_expression(key, parameters, locals, context, &mut encoded)?;
                    if jit_expression_kind(&encoded)?.0 != JitKind::String {
                        return None;
                    }
                    output.extend(encoded);
                }
                output.push(operation.into());
            }
            Expr::Call(call)
                if object_same_value(call, parameters, locals, context.helpers).is_some() =>
            {
                let (left, right) =
                    object_same_value(call, parameters, locals, context.helpers)?;
                let mut encoded_left = Vec::new();
                let mut encoded_right = Vec::new();
                encode_expression(left, parameters, locals, context, &mut encoded_left)?;
                encode_expression(right, parameters, locals, context, &mut encoded_right)?;
                let left_kind = jit_expression_kind(&encoded_left)?.0;
                let right_kind = jit_expression_kind(&encoded_right)?.0;
                output.extend(encoded_left);
                output.extend(encoded_right);
                output.push(
                    if left_kind != right_kind {
                        "strictfalse"
                    } else {
                        match left_kind {
                            JitKind::Number => "numsame",
                            JitKind::Boolean => "==",
                            JitKind::String => "strsame",
                            JitKind::Dynamic => return None,
                            JitKind::Array | JitKind::Dictionary => "refsame",
                        }
                    }
                    .into(),
                );
            }
            Expr::Call(call)
                if array_constructor(call, parameters, locals, context.helpers).is_some() =>
            {
                match array_constructor(call, parameters, locals, context.helpers)? {
                    "of" => {
                        output.push("arrayempty".into());
                        let mut prefix = None;
                        for argument in &call.args {
                            append_array_element(
                                argument,
                                parameters,
                                locals,
                                context,
                                &mut prefix,
                                output,
                            )?;
                        }
                    }
                    "from" => {
                        let [argument] = call.args.as_slice() else {
                            return None;
                        };
                        if argument.spread.is_some() {
                            return None;
                        }
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        match jit_expression_kind(&encoded)?.0 {
                            JitKind::Array => {
                                output.extend(encoded);
                                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                                output.push(format!("c{:016x}", f64::INFINITY.to_bits()));
                                output.push("arrayslice".into());
                            }
                            JitKind::String => {
                                output.extend(encoded);
                                output.push("strarray".into());
                            }
                            JitKind::Number
                            | JitKind::Boolean
                            | JitKind::Dynamic
                            | JitKind::Dictionary => return None,
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Expr::Call(call)
                if array_predicate(call, parameters, locals, context.helpers).is_some() =>
            {
                let mut encoded = Vec::new();
                encode_expression(
                    array_predicate(call, parameters, locals, context.helpers)?,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                let kind = jit_expression_kind(&encoded)?.0;
                output.extend(encoded);
                output.push(
                    match kind {
                        JitKind::Array => "isarray",
                        JitKind::Dynamic => "dynisarray",
                        _ => "isnotarray",
                    }
                    .into(),
                );
            }
            Expr::Call(call) if array_method(call, parameters, locals).is_some() => {
                let (method, receiver) = array_method(call, parameters, locals)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                if encoded.len() == 1 && jit_dynamic_array_argument(&encoded[0]) {
                    encoded.push("untagarray".into());
                }
                if jit_expression_kind(&encoded)?.0 != JitKind::Array {
                    return None;
                }
                let dynamic_array = encoded.last().is_some_and(|token| token == "untagarray");
                if dynamic_array {
                    encoded.pop();
                }
                let prefix = array_prefix(&encoded)
                    .or_else(|| dynamic_array.then_some("dynamic"))?;
                if dynamic_array
                    && !matches!(
                        method,
                        "join"
                            | "toString"
                            | "slice"
                            | "concat"
                            | "at"
                            | "includes"
                            | "indexOf"
                            | "lastIndexOf"
                            | "toReversed"
                            | "reverse"
                            | "toSorted"
                            | "sort"
                            | "fill"
                            | "copyWithin"
                            | "with"
                            | "push"
                            | "unshift"
                            | "pop"
                            | "shift"
                            | "splice"
                            | "toSpliced"
                            | "some"
                            | "every"
                            | "find"
                            | "findIndex"
                            | "findLast"
                            | "findLastIndex"
                            | "filter"
                            | "map"
                            | "reduce"
                            | "reduceRight"
                    )
                {
                    return None;
                }
                let encoded_receiver = encoded.clone();
                let local_insert = matches!(method, "push" | "unshift")
                    .then(|| match receiver {
                        Expr::Ident(identifier) => locals
                            .get(identifier.sym.as_ref())
                            .and_then(|tokens| tokens.first())
                            .and_then(|token| loop_local_index(token)),
                        _ => None,
                    })
                    .flatten()
                    .filter(|_| !call.args.is_empty());
                if local_insert.is_none() {
                    output.extend(encoded);
                }
                if matches!(method, "reduce" | "reduceRight") {
                    let (callback, initial) = match call.args.as_slice() {
                        [callback] => (callback, None),
                        [callback, initial] => (callback, Some(initial)),
                        _ => return None,
                    };
                    if prefix != "rn" && !dynamic_array {
                        return None;
                    }
                    if callback.spread.is_some()
                        || initial.is_some_and(|initial| initial.spread.is_some())
                    {
                        return None;
                    }
                    if let Some(initial) = initial {
                        encode_number(
                            initial.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    } else {
                        output.push("c0000000000000000".into());
                    }
                    let reducer = (!dynamic_array).then(|| {
                        numeric_reducer(callback.expr.as_ref(), parameters, locals, context)
                    });
                    if let Some(operation) = reducer.flatten() {
                        output.push(format!(
                            "rnreduce{}{operation}{}",
                            if method == "reduceRight" { "right" } else { "" },
                            if initial.is_some() { "" } else { "0" }
                        ));
                    } else {
                        let (mut callback, mut kind, captures) = encode_numeric_jit_callback(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            4,
                            if dynamic_array && initial.is_none() {
                                None
                            } else {
                                Some(3)
                            },
                            ("a", "rn", dynamic_array),
                        )?;
                        if dynamic_array && initial.is_none() && kind != JitKind::Dynamic {
                            callback.push(
                                match kind {
                                    JitKind::Number => "tagnum",
                                    JitKind::Boolean => "tagbool",
                                    JitKind::String => "tagstr",
                                    _ => return None,
                                }
                                .into(),
                            );
                            kind = JitKind::Dynamic;
                        }
                        if kind
                            != if dynamic_array && initial.is_none() {
                                JitKind::Dynamic
                            } else {
                                JitKind::Number
                            }
                        {
                            return None;
                        }
                        encode_string(&callback.join(","), output)?;
                        let captured = !captures.is_empty();
                        if captured {
                            append_jit_captures(captures, output);
                        }
                        output.push(format!(
                            "{}reduce{}jit{}{}",
                            if dynamic_array { "dynarray" } else { "rn" },
                            if method == "reduceRight" { "right" } else { "" },
                            if captured { "c" } else { "" },
                            if initial.is_some() { "" } else { "0" }
                        ));
                    }
                } else if method == "map" {
                    let [callback] = call.args.as_slice() else {
                        return None;
                    };
                    if let Some(target) = primitive_conversion_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(format!(
                            "{}mapto{target}",
                            if dynamic_array { "dynarray" } else { prefix }
                        ));
                    } else if prefix != "rn" {
                        if let Some(operation) = primitive_unary_map(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            prefix == "rb",
                        ) {
                            if dynamic_array && operation != "identity" {
                                return None;
                            }
                            output.push(if dynamic_array {
                                "dynarraymapidentity".into()
                            } else {
                                format!("{prefix}map{operation}")
                            });
                        } else {
                            let element_prefix = if prefix == "rb" { "b" } else { "s" };
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                (element_prefix, prefix, dynamic_array),
                            )?;
                            let target = match kind {
                                JitKind::Number => "n",
                                JitKind::Boolean => "b",
                                JitKind::String => "s",
                                JitKind::Dynamic => return None,
                                JitKind::Array | JitKind::Dictionary => return None,
                            };
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(format!(
                                "{}mapjit{target}{}",
                                if dynamic_array { "dynarray" } else { prefix },
                                if captured { "c" } else { "" }
                            ));
                        }
                    } else if let Some(operation) = encode_numeric_conditional_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        output,
                    ) {
                        output.push(operation);
                    } else if let Some(operation) = numeric_unary_map(
                        callback.expr.as_ref(), parameters, locals, context,
                    ) {
                        output.push(format!("rnmap{operation}"));
                    } else if let Some((operation, reverse)) = numeric_index_map(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(format!(
                            "rnmapindex{}{operation}",
                            if reverse { "r" } else { "" }
                        ));
                    } else {
                        let mut operand = Vec::new();
                        if let Some((operation, reverse)) = encode_numeric_map_operand(
                            callback.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut operand,
                        ) {
                            output.extend(operand);
                            output.push(format!(
                                "rnmap{}{operation}",
                                if reverse { "r" } else { "" }
                            ));
                        } else {
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                ("a", "rn", false),
                            )?;
                            if kind != JitKind::Number {
                                return None;
                            }
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(if captured {
                                "rnmapjitc".into()
                            } else {
                                "rnmapjit".into()
                            });
                        }
                    }
                } else if matches!(
                    method,
                    "some"
                        | "every"
                        | "find"
                        | "findIndex"
                        | "findLast"
                        | "findLastIndex"
                        | "filter"
                ) {
                    let [callback] = call.args.as_slice() else {
                        return None;
                    };
                    let method = match method {
                        "findIndex" => "findindex",
                        "findLast" => "findlast",
                        "findLastIndex" => "findlastindex",
                        method => method,
                    };
                    if primitive_truthy_callback(
                        callback.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                    ) {
                        output.push(if dynamic_array {
                            format!("dynarray{method}truthy")
                        } else {
                            format!("{prefix}{method}truthy")
                        });
                    } else {
                        let mut operand = Vec::new();
                        let operation = if prefix == "rn" {
                            encode_numeric_quantifier_operand(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut operand,
                            )
                        } else {
                            encode_primitive_comparison_operand(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                match prefix {
                                    "rs" => JitKind::String,
                                    "rb" => JitKind::Boolean,
                                    "dynamic" => JitKind::Dynamic,
                                    _ => return None,
                                },
                                &mut operand,
                            )
                        };
                        if let Some(operation) = operation {
                            output.extend(operand);
                            output.push(format!(
                                "{}{method}{operation}",
                                if dynamic_array { "dynarray" } else { prefix }
                            ));
                        } else {
                            let element_prefix = match prefix {
                                "rn" => "a",
                                "rb" => "b",
                                "rs" => "s",
                                _ if dynamic_array => "u",
                                _ => return None,
                            };
                            let (callback, kind, captures) = encode_numeric_jit_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                3,
                                Some(2),
                                (element_prefix, prefix, dynamic_array),
                            )?;
                            if !matches!(kind, JitKind::Number | JitKind::Boolean) {
                                return None;
                            }
                            encode_string(&callback.join(","), output)?;
                            let captured = !captures.is_empty();
                            if captured {
                                append_jit_captures(captures, output);
                            }
                            output.push(format!(
                                "{}{method}jit{}",
                                if dynamic_array { "dynarray" } else { prefix },
                                if captured { "c" } else { "" }
                            ));
                        }
                    }
                } else if matches!(method, "join" | "toString") {
                    match call.args.as_slice() {
                        [] => encode_string(",", output)?,
                        [separator] if method == "join" => {
                            let mut encoded_separator = Vec::new();
                            encode_expression(
                                separator.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded_separator,
                            )?;
                            append_string(encoded_separator, output)?;
                        }
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarrayjoin".into()
                    } else {
                        format!("{prefix}join")
                    });
                } else if matches!(method, "push" | "unshift") {
                    if call.args.is_empty() {
                        output.push("arraylen".into());
                    }
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    let arguments: Vec<_> = if method == "unshift" {
                        call.args.iter().rev().collect()
                    } else {
                        call.args.iter().collect()
                    };
                    for (index, argument) in arguments.iter().enumerate() {
                        if index != 0 && local_insert.is_none() {
                            output.extend(encoded_receiver.iter().cloned());
                        }
                        let mut encoded_value = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_value,
                        )?;
                        if jit_expression_kind(&encoded_value)?.0 != expected {
                            return None;
                        }
                        output.extend(encoded_value);
                        output.push(if dynamic_array {
                            format!("dynarray{method}")
                        } else {
                            local_insert.map_or_else(
                                || format!("{prefix}{method}"),
                                |local| format!("{prefix}l{method}{local}"),
                            )
                        });
                        if index + 1 != arguments.len() {
                            output.push("drop".into());
                        }
                    }
                } else if matches!(method, "pop" | "shift") {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push(if dynamic_array {
                        format!("dynarray{method}")
                    } else {
                        format!("{prefix}{method}")
                    });
                } else if matches!(method, "splice" | "toSpliced") {
                    let scalar_kind = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    match call.args.as_slice() {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c0000000000000000".into());
                        }
                        [start] => {
                            encode_number(
                                start.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, delete_count, ..] => {
                            encode_number(
                                start.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                            encode_number(
                                delete_count.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                        }
                    }
                    output.extend(encoded_receiver.iter().cloned());
                    output.push("c0000000000000000".into());
                    output.push("c0000000000000000".into());
                    output.push(if dynamic_array {
                        "dynarrayslice".into()
                    } else {
                        "arrayslice".into()
                    });
                    for argument in call.args.iter().skip(2) {
                        let mut encoded_value = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_value,
                        )?;
                        if jit_expression_kind(&encoded_value)?.0 != scalar_kind {
                            return None;
                        }
                        output.extend(encoded_value);
                        output.push(if dynamic_array {
                            "dynarrayappend".into()
                        } else {
                            format!("{prefix}append")
                        });
                    }
                    output.push(
                        if dynamic_array {
                            if method == "splice" {
                                "dynarraysplice"
                            } else {
                                "dynarraytospliced"
                            }
                        } else if method == "splice" {
                            "arraysplice"
                        } else {
                            "arraytospliced"
                        }
                        .into(),
                    );
                } else if method == "concat" {
                    if call.args.is_empty() {
                        output.push("c0000000000000000".into());
                        output.push("c7ff0000000000000".into());
                        output.push(
                            if dynamic_array {
                                "dynarrayslice"
                            } else {
                                "arrayslice"
                            }
                            .into(),
                        );
                    }
                    if dynamic_array {
                        for argument in &call.args {
                            let mut encoded_argument = Vec::new();
                            encode_expression(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded_argument,
                            )?;
                            if encoded_argument.pop().as_deref() != Some("untagarray")
                                || jit_expression_kind(&encoded_argument)?.0 != JitKind::Dynamic
                            {
                                return None;
                            }
                            output.extend(encoded_argument);
                            output.push("dynarrayconcat".into());
                        }
                        return Some(());
                    }
                    let scalar_kind = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        _ => return None,
                    };
                    for argument in &call.args {
                        let mut encoded_argument = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded_argument,
                        )?;
                        match jit_expression_kind(&encoded_argument)?.0 {
                            JitKind::Array if array_prefix(&encoded_argument)? == prefix => {
                                output.extend(encoded_argument);
                                output.push("arrayconcat".into());
                            }
                            kind if kind == scalar_kind => {
                                output.extend(encoded_argument);
                                output.push(format!("{prefix}append"));
                            }
                            _ => return None,
                        }
                    }
                } else if matches!(method, "toReversed" | "reverse") {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push(if method == "reverse" {
                        if dynamic_array {
                            "dynarrayreverse".into()
                        } else {
                            "arrayreverse".into()
                        }
                    } else if dynamic_array {
                        "dynarrayreversed".into()
                    } else {
                        "arrayreversed".into()
                    });
                } else if matches!(method, "toSorted" | "sort") {
                    let suffix = if method == "sort" { "sort" } else { "sorted" };
                    match call.args.as_slice() {
                        [] => output.push(if dynamic_array {
                            format!("dynarray{suffix}")
                        } else {
                            format!("{prefix}{suffix}")
                        }),
                        [callback] if callback.spread.is_none() && prefix == "rn" => output.push(
                            format!(
                                "rn{suffix}{}",
                                if numeric_sort_callback(
                                    callback.expr.as_ref(),
                                    parameters,
                                    locals,
                                    context,
                                )? {
                                    "desc"
                                } else {
                                    "asc"
                                }
                            ),
                        ),
                        [callback] if callback.spread.is_none() && prefix == "rs" => {
                            if string_sort_callback(
                                callback.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                            )? {
                                output.push(format!("rs{suffix}desc"));
                            } else {
                                output.push(format!("rs{suffix}"));
                            }
                        }
                        _ => return None,
                    }
                } else if method == "fill" {
                    let [value, range @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    if range.len() > 2 {
                        return None;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(value.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if jit_expression_kind(&encoded)?.0 != expected {
                        return None;
                    }
                    output.extend(encoded);
                    match range {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c7ff0000000000000".into());
                        }
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => unreachable!(),
                    }
                    output.push(if dynamic_array {
                        "dynarrayfill".into()
                    } else {
                        format!("{prefix}fill")
                    });
                } else if method == "copyWithin" {
                    match call.args.as_slice() {
                        [target, start] => {
                            encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [target, start, end] => {
                            encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarraycopywithin".into()
                    } else {
                        "arraycopywithin".into()
                    });
                } else if method == "with" {
                    let [index, value] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(index.expr.as_ref(), parameters, locals, context, output)?;
                    let mut encoded = Vec::new();
                    encode_expression(value.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if jit_expression_kind(&encoded)?.0 != expected {
                        return None;
                    }
                    output.extend(encoded);
                    output.push(if dynamic_array {
                        "dynarraywith".into()
                    } else {
                        format!("{prefix}with")
                    });
                } else if method == "slice" {
                    match call.args.as_slice() {
                        [] => {
                            output.push("c0000000000000000".into());
                            output.push("c7ff0000000000000".into());
                        }
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            output.push("c7ff0000000000000".into());
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                        }
                        _ => return None,
                    }
                    output.push(
                        if dynamic_array {
                            "dynarrayslice"
                        } else {
                            "arrayslice"
                        }
                        .into(),
                    );
                } else if method == "at" {
                    match call.args.as_slice() {
                        [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                        [index] => encode_number(
                            index.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                    output.push(if dynamic_array {
                        "dynarrayat".into()
                    } else {
                        format!("{prefix}at")
                    });
                } else {
                    let ([needle] | [needle, ..]) = call.args.as_slice() else {
                        return None;
                    };
                    let mut needle = {
                        let mut value = Vec::new();
                        encode_expression(
                            needle.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut value,
                        )?;
                        value
                    };
                    let expected = match prefix {
                        "rn" => JitKind::Number,
                        "rb" => JitKind::Boolean,
                        "rs" => JitKind::String,
                        "dynamic" => JitKind::Dynamic,
                        _ => return None,
                    };
                    if jit_expression_kind(&needle)?.0 != expected {
                        return None;
                    }
                    output.append(&mut needle);
                    match call.args.get(1) {
                        Some(from) => encode_number(
                            from.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        None => output.push(format!(
                            "c{:016x}",
                            if method == "lastIndexOf" {
                                f64::INFINITY
                            } else {
                                0.0
                            }
                            .to_bits()
                        )),
                    }
                    if call.args.len() > 2 {
                        return None;
                    }
                    let operation = match method {
                        "includes" => "includes",
                        "indexOf" => "indexof",
                        "lastIndexOf" => "lastindexof",
                        _ => return None,
                    };
                    output.push(if dynamic_array {
                        format!("dynarray{operation}")
                    } else {
                        format!("{prefix}{operation}")
                    });
                }
            }
            Expr::Call(call) if primitive_value_of(call).is_some() => {
                encode_expression(
                    primitive_value_of(call)?,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
            }
            Expr::Call(call) if string_method(call, parameters, locals).is_some() => {
                let (operation, receiver) = string_method(call, parameters, locals)?;
                let mut encoded = Vec::new();
                encode_expression(receiver, parameters, locals, context, &mut encoded)?;
                match jit_expression_kind(&encoded)?.0 {
                    JitKind::String => output.extend(encoded),
                    JitKind::Dynamic => {
                        output.extend(encoded);
                        output.push("untagstr".into());
                    }
                    _ => return None,
                }
                if operation == "concat" {
                    for argument in &call.args {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                        output.push("concat".into());
                    }
                    return (output.len() <= 128).then_some(());
                } else if matches!(
                    operation,
                    "tolowercase"
                        | "touppercase"
                        | "iswellformed"
                        | "towellformed"
                        | "trim"
                        | "trimstart"
                        | "trimend"
                ) {
                    if !call.args.is_empty() {
                        return None;
                    }
                } else if operation == "normalize" {
                    match call.args.as_slice() {
                        [] => output.push("t4e4643".into()),
                        [form] => {
                            let mut encoded = Vec::new();
                            encode_expression(
                                form.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            append_string(encoded, output)?;
                        }
                        _ => return None,
                    }
                } else if operation == "repeat" {
                    let [count] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(count.expr.as_ref(), parameters, locals, context, output)?;
                } else if matches!(operation, "replace" | "replaceall") {
                    let [search, replacement] = call.args.as_slice() else {
                        return None;
                    };
                    for argument in [search, replacement] {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                    }
                } else if matches!(operation, "charat" | "charcodeat" | "at" | "codepointat") {
                    match call.args.as_slice() {
                        [] => output.push("c0000000000000000".into()),
                        [index] => {
                            encode_number(index.expr.as_ref(), parameters, locals, context, output)?
                        }
                        _ => return None,
                    }
                } else if matches!(operation, "padstart" | "padend") {
                    let [target, pad @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(target.expr.as_ref(), parameters, locals, context, output)?;
                    match pad {
                        [] => output.push("t20".into()),
                        [pad] => {
                            let mut encoded = Vec::new();
                            encode_expression(
                                pad.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            append_string(encoded, output)?;
                        }
                        _ => return None,
                    }
                } else if matches!(operation, "slice" | "substring") {
                    match call.args.as_slice() {
                        [] => output.push("c0000000000000000".into()),
                        [start] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?
                        }
                        [start, end] => {
                            encode_number(start.expr.as_ref(), parameters, locals, context, output)?;
                            encode_number(end.expr.as_ref(), parameters, locals, context, output)?;
                            output.push(format!("{operation}2"));
                            return Some(());
                        }
                        _ => return None,
                    }
                } else if operation == "split" {
                    let [separator, limit @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(
                        separator.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_string(encoded, output)?;
                    match limit {
                        [] => output.push("c7ff0000000000000".into()),
                        [limit] => encode_number(
                            limit.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                } else {
                    let [search, position @ ..] = call.args.as_slice() else {
                        return None;
                    };
                    let mut encoded = Vec::new();
                    encode_expression(search.expr.as_ref(), parameters, locals, context, &mut encoded)?;
                    append_string(encoded, output)?;
                    match position {
                        [] => {}
                        [position] => {
                            encode_number(position.expr.as_ref(), parameters, locals, context, output)?;
                            output.push(format!("{operation}2"));
                            return Some(());
                        }
                        _ => return None,
                    }
                }
                output.push(operation.into());
            }
            Expr::Call(call) if number_parser(call, parameters, locals, context.helpers).is_some() => {
                let operation = number_parser(call, parameters, locals, context.helpers)?;
                let radix = if let Some((value, radix)) = call.args.split_first() {
                    let mut encoded = Vec::new();
                    encode_expression(
                        value.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_string(encoded, output)?;
                    radix
                } else {
                    encode_string("undefined", output)?;
                    &[]
                };
                if operation == "parsefloat" {
                    if !radix.is_empty() {
                        return None;
                    }
                } else {
                    match radix {
                        [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                        [radix] => encode_number(
                            radix.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            output,
                        )?,
                        _ => return None,
                    }
                }
                output.push(operation.into());
            }
            Expr::Call(call)
                if number_predicate(call, parameters, locals, context.helpers).is_some() =>
            {
                let (operation, coercive) =
                    number_predicate(call, parameters, locals, context.helpers)?;
                let [argument] = call.args.as_slice() else {
                    return None;
                };
                let mut encoded = Vec::new();
                encode_expression(
                    argument.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                if coercive {
                    append_number(encoded, output)?;
                    output.push(operation.into());
                } else if jit_expression_kind(&encoded)?.0 == JitKind::Number {
                    output.extend(encoded);
                    output.push(operation.into());
                } else {
                    output.extend(encoded);
                    output.push(format!("c{:016x}", 0.0f64.to_bits()));
                    output.push("strictfalse".into());
                }
            }
            Expr::Call(call)
                if string_static_constructor(call, parameters, locals, context.helpers).is_some() =>
            {
                let operation = match string_static_constructor(
                    call,
                    parameters,
                    locals,
                    context.helpers,
                )? {
                    "fromCharCode" => "fromcharcode",
                    "fromCodePoint" => "fromcodepoint",
                    _ => unreachable!(),
                };
                if call.args.is_empty() {
                    encode_string("", output)?;
                }
                for (index, argument) in call.args.iter().enumerate() {
                    let mut encoded = Vec::new();
                    encode_expression(
                        argument.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    append_number(encoded, output)?;
                    output.push(operation.into());
                    if index != 0 {
                        output.push("concat".into());
                    }
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "String")
                ) =>
            {
                if parameters.contains_key("String") || locals.contains_key("String") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => encode_string("", output)?,
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_string(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Number")
                ) =>
            {
                if parameters.contains_key("Number") || locals.contains_key("Number") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_number(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call)
                if matches!(
                    &call.callee,
                    Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Boolean")
                ) =>
            {
                if parameters.contains_key("Boolean") || locals.contains_key("Boolean") {
                    return None;
                }
                match call.args.as_slice() {
                    [] => output.push(format!("c{:016x}", 0.0f64.to_bits())),
                    [argument] if argument.spread.is_none() => {
                        let mut encoded = Vec::new();
                        encode_expression(
                            argument.expr.as_ref(),
                            parameters,
                            locals,
                            context,
                            &mut encoded,
                        )?;
                        append_boolean(encoded, output)?;
                    }
                    _ => return None,
                }
            }
            Expr::Call(call) if time_method(call, parameters, locals).is_some() => {
                let method = time_method(call, parameters, locals)?;
                if method == "processuptime" {
                    output.push("performancenow".into());
                    output.push(format!("c{:016x}", 1_000.0f64.to_bits()));
                    output.push("/".into());
                } else {
                    output.push(method.into());
                }
            }
            Expr::Call(call) if math_method(call, parameters, locals).is_some() => {
                let method = math_method(call, parameters, locals)?;
                if method == "random" {
                    if !call.args.is_empty() {
                        return None;
                    }
                    output.push("random".into());
                } else if matches!(
                    method,
                    "abs"
                        | "acos"
                        | "acosh"
                        | "asin"
                        | "asinh"
                        | "atan"
                        | "atanh"
                        | "cbrt"
                        | "ceil"
                        | "clz32"
                        | "cos"
                        | "cosh"
                        | "exp"
                        | "expm1"
                        | "floor"
                        | "fround"
                        | "log"
                        | "log1p"
                        | "log2"
                        | "log10"
                        | "round"
                        | "sign"
                        | "sin"
                        | "sinh"
                        | "sqrt"
                        | "tan"
                        | "tanh"
                        | "trunc"
                ) {
                    let [argument] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(argument.expr.as_ref(), parameters, locals, context, output)?;
                    output.push(method.into());
                } else if method == "pow" {
                    let [base, exponent] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(base.expr.as_ref(), parameters, locals, context, output)?;
                    encode_number(exponent.expr.as_ref(), parameters, locals, context, output)?;
                    output.push("pow".into());
                } else if matches!(method, "atan2" | "imul") {
                    let [y, x] = call.args.as_slice() else {
                        return None;
                    };
                    encode_number(y.expr.as_ref(), parameters, locals, context, output)?;
                    encode_number(x.expr.as_ref(), parameters, locals, context, output)?;
                    output.push(method.into());
                } else if call.args.is_empty() {
                    let value = match method {
                        "min" => f64::INFINITY,
                        "max" => f64::NEG_INFINITY,
                        "hypot" => 0.0,
                        _ => return None,
                    };
                    output.push(format!("c{:016x}", value.to_bits()));
                } else {
                    for (index, argument) in call.args.iter().enumerate() {
                        if argument.spread.is_some() {
                            let mut encoded = Vec::new();
                            encode_expression(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                &mut encoded,
                            )?;
                            if jit_expression_kind(&encoded)?.0 != JitKind::Array
                                || array_prefix(&encoded)? != "rn"
                            {
                                return None;
                            }
                            output.extend(encoded);
                            output.push(format!("rn{method}"));
                        } else {
                            encode_number(
                                argument.expr.as_ref(),
                                parameters,
                                locals,
                                context,
                                output,
                            )?;
                        }
                        if index != 0 {
                            output.push(method.into());
                        }
                    }
                    if method == "hypot"
                        && call.args.len() == 1
                        && call.args[0].spread.is_none()
                    {
                        output.push("abs".into());
                    }
                }
            }
            Expr::Call(call) => {
                encode_helper_call(call, parameters, locals, context, output)?;
            }
            Expr::Cond(conditional) => {
                encode_condition(conditional.test.as_ref(), parameters, locals, context, output)?;
                let narrowed = narrowed_locals(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context.helpers,
                );
                let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
                let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
                let mut consequent = Vec::new();
                let mut alternate = Vec::new();
                encode_expression(
                    conditional.cons.as_ref(),
                    parameters,
                    consequent_locals,
                    context,
                    &mut consequent,
                )?;
                encode_expression(
                    conditional.alt.as_ref(),
                    parameters,
                    alternate_locals,
                    context,
                    &mut alternate,
                )?;
                if boolean_literal(conditional.cons.as_ref()) {
                    consequent.push("asbool".into());
                }
                if boolean_literal(conditional.alt.as_ref()) {
                    alternate.push("asbool".into());
                }
                normalize_callable_branches([&mut consequent, &mut alternate])?;
                output.push("if".into());
                output.extend(consequent);
                output.push("else".into());
                output.extend(alternate);
                output.push("end".into());
            }
            _ => return None,
        }
        (output.len() <= 256).then_some(())
    }

    fn encode_string(value: &str, output: &mut Vec<String>) -> Option<()> {
        if value.as_bytes().contains(&0) {
            return None;
        }
        output.push(format!(
            "t{}",
            value
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        Some(())
    }

    fn encode_condition(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let Expr::Bin(binary) = expression {
            let operator = match binary.op {
                BinaryOp::Lt => Some("<"),
                BinaryOp::LtEq => Some("<="),
                BinaryOp::Gt => Some(">"),
                BinaryOp::GtEq => Some(">="),
                BinaryOp::EqEq | BinaryOp::EqEqEq => Some("=="),
                BinaryOp::NotEq | BinaryOp::NotEqEq => Some("!="),
                _ => None,
            };
            if let Some(operator) = operator {
                let mut left = Vec::new();
                let mut right = Vec::new();
                encode_expression(binary.left.as_ref(), parameters, locals, context, &mut left)?;
                encode_expression(binary.right.as_ref(), parameters, locals, context, &mut right)?;
                let left_kind = jit_expression_kind(&left)?.0;
                let right_kind = jit_expression_kind(&right)?.0;
                let strict = matches!(binary.op, BinaryOp::EqEqEq | BinaryOp::NotEqEq);
                if left_kind == JitKind::Dynamic || right_kind == JitKind::Dynamic {
                    append_dynamic(left, output)?;
                    append_dynamic(right, output)?;
                    output.push(
                        match binary.op {
                            BinaryOp::Lt => "dynlt",
                            BinaryOp::LtEq => "dynlte",
                            BinaryOp::Gt => "dyngt",
                            BinaryOp::GtEq => "dyngte",
                            BinaryOp::EqEq => "dyneq",
                            BinaryOp::NotEq => "dynne",
                            BinaryOp::EqEqEq => "dynseq",
                            BinaryOp::NotEqEq => "dynsne",
                            _ => unreachable!(),
                        }
                        .into(),
                    );
                } else if strict && left_kind != right_kind {
                    output.extend(left);
                    output.extend(right);
                    output.push(
                        if binary.op == BinaryOp::NotEqEq {
                            "stricttrue"
                        } else {
                            "strictfalse"
                        }
                        .into(),
                    );
                } else if left_kind == JitKind::String && right_kind == JitKind::String {
                    output.extend(left);
                    output.extend(right);
                    output.push("strcmp".into());
                    output.push(format!("c{:016x}", 0.0f64.to_bits()));
                    output.push(operator.into());
                } else {
                    append_number(left, output)?;
                    append_number(right, output)?;
                    output.push(operator.into());
                }
                return (output.len() <= 128).then_some(());
            }
        }
        let mut encoded = Vec::new();
        encode_expression(expression, parameters, locals, context, &mut encoded)?;
        append_boolean(encoded, output)?;
        (output.len() <= 256).then_some(())
    }

    enum NumericBody<'a> {
        Expression(&'a Expr),
        Statements(&'a [Stmt]),
    }

    fn boolean_literal(expression: &Expr) -> bool {
        let mut expression = expression;
        while let Expr::Paren(parenthesized) = expression {
            expression = parenthesized.expr.as_ref();
        }
        matches!(expression, Expr::Lit(Lit::Bool(_)))
    }

    fn dynamic_primitive_narrowing(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(String, JitKind, bool)> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let equal = match binary.op {
            BinaryOp::EqEq | BinaryOp::EqEqEq => true,
            BinaryOp::NotEq | BinaryOp::NotEqEq => false,
            _ => return None,
        };
        let probe = |typeof_expression: &Expr, type_expression: &Expr| {
            let Expr::Unary(typeof_expression) = typeof_expression else {
                return None;
            };
            if typeof_expression.op != UnaryOp::TypeOf {
                return None;
            }
            let Expr::Ident(name) = typeof_expression.arg.as_ref() else {
                return None;
            };
            let Expr::Lit(Lit::Str(ty)) = type_expression else {
                return None;
            };
            let selected = match ty.value.to_string_lossy().as_ref() {
                "number" => JitKind::Number,
                "boolean" => JitKind::Boolean,
                "string" => JitKind::String,
                _ => return None,
            };
            (jit_expression_kind(locals.get(name.sym.as_ref())?)?.0 == JitKind::Dynamic)
                .then(|| (name.sym.to_string(), selected))
        };
        let (name, selected) = probe(binary.left.as_ref(), binary.right.as_ref())
            .or_else(|| probe(binary.right.as_ref(), binary.left.as_ref()))?;
        Some((name, selected, equal))
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_primitive_locals(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let (name, selected, equal) = dynamic_primitive_narrowing(expression, locals)?;
        let source = locals.get(&name)?;
        let mut available = source
            .iter()
            .filter_map(|token| match token.as_str() {
                "tagnum" => Some(JitKind::Number),
                "tagbool" => Some(JitKind::Boolean),
                "tagstr" => Some(JitKind::String),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        for kind in source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))
            .into_iter()
            .flat_map(str::bytes)
        {
            available.insert(match kind {
                b'n' => JitKind::Number,
                b'b' => JitKind::Boolean,
                b's' => JitKind::String,
                _ => unreachable!(),
            });
        }
        if available.is_empty() {
            available.extend(if source
                .iter()
                .any(|token| token.strip_prefix("ld").is_some())
            {
                [JitKind::Number, JitKind::Boolean, JitKind::String].as_slice()
            } else {
                [JitKind::Number, JitKind::String].as_slice()
            });
        }
        for token in source {
            available.remove(&match token.as_str() {
                "notnum" => JitKind::Number,
                "notbool" => JitKind::Boolean,
                "notstr" => JitKind::String,
                _ => continue,
            });
        }
        if !available.contains(&selected) {
            return None;
        }
        let narrow = |matches: bool| {
            let mut value = source.clone();
            let kind = if matches {
                selected
            } else {
                value.push(match selected {
                    JitKind::Number => "notnum",
                    JitKind::Boolean => "notbool",
                    JitKind::String => "notstr",
                    _ => return None,
                }
                .into());
                let remaining = available
                    .iter()
                    .copied()
                    .filter(|kind| *kind != selected)
                    .collect::<Vec<_>>();
                if remaining.len() != 1 {
                    let mut narrowed = locals.clone();
                    narrowed.insert(name.clone(), value);
                    return Some(narrowed);
                }
                remaining[0]
            };
            value.push(match kind {
                JitKind::Number => "untagnum",
                JitKind::Boolean => "untagbool",
                JitKind::String => "untagstr",
                _ => return None,
            }
            .into());
            let mut narrowed = locals.clone();
            narrowed.insert(name.clone(), value);
            Some(narrowed)
        };
        Some((narrow(equal)?, narrow(!equal)?))
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_array_locals(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let (call, positive) = match expression {
            Expr::Call(call) => (call, true),
            Expr::Unary(unary) if unary.op == UnaryOp::Bang => {
                let Expr::Call(call) = unary.arg.as_ref() else {
                    return None;
                };
                (call, false)
            }
            _ => return None,
        };
        let Expr::Ident(identifier) = array_predicate(call, parameters, locals, helpers)? else {
            return None;
        };
        let source = locals.get(identifier.sym.as_ref())?;
        let kinds = source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))?;
        let not_array = source.iter().any(|token| token == "notarray");
        let arrays = kinds
            .bytes()
            .filter(|kind| matches!(kind, b'N' | b'B' | b'S' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        if not_array || arrays.is_empty() {
            return None;
        }
        let mut array_value = source.clone();
        array_value.push(
            if arrays.iter().all(|kind| matches!(kind, b'N' | b'X')) {
                "untagarrayn"
            } else if arrays.iter().all(|kind| matches!(kind, b'B' | b'Y')) {
                "untagarrayb"
            } else if arrays.iter().all(|kind| matches!(kind, b'S' | b'Z')) {
                "untagarrays"
            } else {
                match arrays.as_slice() {
                [b'N'] => "untagrn",
                [b'B'] => "untagrb",
                [b'S'] => "untagrs",
                    [b'T' | b'X' | b'Y' | b'Z'] => "untagtuple",
                _ => "untagarray",
                }
            }
            .into(),
        );
        let remaining = kinds
            .bytes()
            .filter(|kind| !matches!(kind, b'N' | b'B' | b'S' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        let mut other_value = source.clone();
        if remaining.len() == 1 {
            other_value.push(
                match remaining[0] {
                    b'n' => "untagnum",
                    b'b' => "untagbool",
                    b's' => "untagstr",
                    b'D' => "untagdn",
                    b'E' => "untagdb",
                    b'F' => "untagds",
                    b'O' => "untagobject",
                    _ => return None,
                }
                .into(),
            );
        } else {
            other_value.push("notarray".into());
        }
        let narrowed = |value: Vec<String>| {
            let mut narrowed = locals.clone();
            narrowed.insert(identifier.sym.to_string(), value);
            narrowed
        };
        let branches = (narrowed(array_value), narrowed(other_value));
        Some(if positive {
            branches
        } else {
            (branches.1, branches.0)
        })
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_object_locals(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        let expression = match expression {
            Expr::Paren(expression) => expression.expr.as_ref(),
            expression => expression,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let equal = match binary.op {
            BinaryOp::EqEq | BinaryOp::EqEqEq => true,
            BinaryOp::NotEq | BinaryOp::NotEqEq => false,
            _ => return None,
        };
        let probe = |typeof_expression: &Expr, type_expression: &Expr| {
            let Expr::Unary(typeof_expression) = typeof_expression else {
                return None;
            };
            if typeof_expression.op != UnaryOp::TypeOf {
                return None;
            }
            let Expr::Ident(name) = typeof_expression.arg.as_ref() else {
                return None;
            };
            let Expr::Lit(Lit::Str(ty)) = type_expression else {
                return None;
            };
            (ty.value == *"object").then(|| name.sym.to_string())
        };
        let identifier = probe(binary.left.as_ref(), binary.right.as_ref())
            .or_else(|| probe(binary.right.as_ref(), binary.left.as_ref()))?;
        let source = locals.get(&identifier)?;
        let kinds = source
            .iter()
            .find_map(|token| jit_dynamic_argument(token).map(|(_, kinds)| kinds))?;
        let not_array = source.iter().any(|token| token == "notarray");
        let not_object = source.iter().any(|token| token == "notobject");
        let aggregates = kinds
            .bytes()
            .filter(|kind| matches!(kind, b'D' | b'E' | b'F' | b'O' | b'T' | b'X' | b'Y' | b'Z'))
            .collect::<Vec<_>>();
        if not_object
            || aggregates.is_empty()
            || (!not_array
                && kinds
                    .bytes()
                    .any(|kind| matches!(kind, b'N' | b'B' | b'S')))
        {
            return None;
        }
        let mut object_value = source.clone();
        object_value.push(
            match aggregates.as_slice() {
                [b'D'] => "untagdn",
                [b'E'] => "untagdb",
                [b'F'] => "untagds",
                [b'O'] => "untagobject",
                [b'T' | b'X' | b'Y' | b'Z'] => "untagtuple",
                _ if aggregates
                    .iter()
                    .all(|kind| matches!(kind, b'D' | b'E' | b'F')) =>
                {
                    "untagdictionary"
                }
                _ => return None,
            }
            .into(),
        );
        let remaining = kinds
            .bytes()
            .filter(|kind| {
                !(matches!(kind, b'D' | b'E' | b'F' | b'O' | b'T' | b'X' | b'Y' | b'Z')
                    || not_array && matches!(kind, b'N' | b'B' | b'S'))
            })
            .collect::<Vec<_>>();
        let mut other_value = source.clone();
        if remaining.len() == 1 {
            other_value.push(
                match remaining[0] {
                    b'n' => "untagnum",
                    b'b' => "untagbool",
                    b's' => "untagstr",
                    _ => return None,
                }
                .into(),
            );
        } else {
            other_value.push("notobject".into());
        }
        let narrowed = |value: Vec<String>| {
            let mut narrowed = locals.clone();
            narrowed.insert(identifier.clone(), value);
            narrowed
        };
        let branches = (narrowed(object_value), narrowed(other_value));
        Some(if equal {
            branches
        } else {
            (branches.1, branches.0)
        })
    }

    #[allow(clippy::type_complexity)]
    fn narrowed_locals(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(
        std::collections::HashMap<String, Vec<String>>,
        std::collections::HashMap<String, Vec<String>>,
    )> {
        narrowed_primitive_locals(expression, locals)
            .or_else(|| narrowed_array_locals(expression, parameters, locals, helpers))
            .or_else(|| narrowed_object_locals(expression, locals))
    }

    fn encode_switch_case_return(
        switch: &thaw_parser::ast::SwitchStmt,
        start: usize,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        switch.cases[start..]
            .iter()
            .find(|case| !case.cons.is_empty())
            .and_then(|case| {
                encode_returning_statements(&case.cons, parameters, locals, context, output)
            })
    }

    fn encode_returning_switch(
        switch: &thaw_parser::ast::SwitchStmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut discriminant = Vec::new();
        encode_expression(
            switch.discriminant.as_ref(),
            parameters,
            locals,
            context,
            &mut discriminant,
        )?;
        let kind = if boolean_literal(switch.discriminant.as_ref()) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&discriminant)?.0
        };
        if matches!(kind, JitKind::Array | JitKind::Dictionary) {
            return None;
        }
        let default = switch.cases.iter().position(|case| case.test.is_none())?;
        let tested = switch
            .cases
            .iter()
            .enumerate()
            .filter_map(|(index, case)| case.test.as_ref().map(|_| index))
            .collect::<Vec<_>>();
        let mut returns = tested
            .iter()
            .copied()
            .chain(std::iter::once(default))
            .map(|index| {
                let mut encoded = Vec::new();
                encode_switch_case_return(
                    switch,
                    index,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                Some(encoded)
            })
            .collect::<Option<Vec<_>>>()?;
        normalize_callable_branches(returns.iter_mut())?;
        output.extend(discriminant);
        let mut branches = 0;
        for (return_index, index) in tested.iter().copied().enumerate() {
            let test = switch.cases[index].test.as_deref()?;
            output.push("dup".into());
            let mut encoded = Vec::new();
            encode_expression(test, parameters, locals, context, &mut encoded)?;
            let test_kind = if boolean_literal(test) {
                JitKind::Boolean
            } else {
                jit_expression_kind(&encoded)?.0
            };
            output.extend(encoded);
            if kind == JitKind::String && test_kind == JitKind::String {
                output.push("strcmp".into());
                output.push(format!("c{:016x}", 0.0f64.to_bits()));
                output.push("==".into());
            } else if kind == test_kind
                && matches!(kind, JitKind::Number | JitKind::Boolean)
            {
                output.push("==".into());
            } else {
                output.push("strictfalse".into());
            }
            output.push("if".into());
            output.extend(returns[return_index].iter().cloned());
            output.push("else".into());
            branches += 1;
        }
        output.extend(returns.last()?.iter().cloned());
        output.extend(std::iter::repeat_n("end".into(), branches));
        output.push("nip".into());
        (output.len() <= 128).then_some(())
    }

    fn encode_returning_statement(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match statement {
            Stmt::Return(returned) => {
                let expression = returned.arg.as_deref()?;
                encode_expression(expression, parameters, locals, context, output)?;
                if boolean_literal(expression) {
                    output.push("asbool".into());
                }
                Some(())
            }
            Stmt::Block(block) => {
                encode_returning_statements(&block.stmts, parameters, locals, context, output)
            }
            Stmt::If(_) => encode_returning_statements(
                std::slice::from_ref(statement),
                parameters,
                locals,
                context,
                output,
            ),
            Stmt::Switch(switch) => {
                encode_returning_switch(switch, parameters, locals, context, output)
            }
            _ => None,
        }
    }

    fn encode_returning_statements(
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        if let Stmt::Return(_) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_returning_statement(first, parameters, locals, context, output);
        }
        if let Stmt::Switch(switch) = first {
            if !rest.is_empty() {
                return None;
            }
            return encode_returning_switch(switch, parameters, locals, context, output);
        }
        let Stmt::If(branch) = first else {
            return None;
        };
        encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
        let narrowed = narrowed_locals(
            branch.test.as_ref(),
            parameters,
            locals,
            context.helpers,
        );
        let consequent_locals = narrowed.as_ref().map_or(locals, |locals| &locals.0);
        let alternate_locals = narrowed.as_ref().map_or(locals, |locals| &locals.1);
        let mut consequent = Vec::new();
        encode_returning_statement(
            branch.cons.as_ref(),
            parameters,
            consequent_locals,
            context,
            &mut consequent,
        )?;
        let mut alternate_output = Vec::new();
        if let Some(alternate) = branch.alt.as_deref() {
            if !rest.is_empty() {
                return None;
            }
            encode_returning_statement(
                alternate,
                parameters,
                alternate_locals,
                context,
                &mut alternate_output,
            )?;
        } else {
            encode_returning_statements(
                rest,
                parameters,
                alternate_locals,
                context,
                &mut alternate_output,
            )?;
        }
        normalize_callable_branches([&mut consequent, &mut alternate_output])?;
        output.push("if".into());
        output.extend(consequent);
        output.push("else".into());
        output.extend(alternate_output);
        output.push("end".into());
        (output.len() <= 256).then_some(())
    }

    enum LocalStep<'a> {
        Declare {
            name: &'a Ident,
            initializer: &'a Expr,
            mutable: bool,
        },
        Assign {
            name: &'a Ident,
            operation: AssignOp,
            value: &'a Expr,
        },
        Update {
            name: &'a Ident,
            operation: UpdateOp,
        },
        Effect(&'a Expr),
    }

    #[derive(Clone, Copy)]
    enum BreakTarget {
        Loop,
        Switch,
    }

    #[derive(Clone, Copy)]
    struct LoopControl {
        break_target: BreakTarget,
        break_finalizer_depth: usize,
        continue_finalizer_depth: usize,
        throw_finalizer_depth: usize,
        catch_active: bool,
        catch_tagged: bool,
    }

    type LoopScope<'a> = (
        &'a std::collections::HashMap<String, String>,
        &'a std::collections::HashMap<String, Vec<String>>,
        &'a std::collections::HashSet<String>,
        &'a std::collections::HashMap<String, JitKind>,
    );

    fn nested_loop_control(finalizer_depth: usize, parent: LoopControl) -> LoopControl {
        LoopControl {
            break_target: BreakTarget::Loop,
            break_finalizer_depth: finalizer_depth,
            continue_finalizer_depth: finalizer_depth,
            ..parent
        }
    }

    fn root_loop_control() -> LoopControl {
        LoopControl {
            break_target: BreakTarget::Loop,
            break_finalizer_depth: 0,
            continue_finalizer_depth: 0,
            throw_finalizer_depth: 0,
            catch_active: false,
            catch_tagged: false,
        }
    }

    fn jit_tokens_may_error(tokens: &[String]) -> bool {
        tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "repeat"
                    | "fromcodepoint"
                    | "normalize"
                    | "tofixed"
                    | "toprecision"
                    | "toexponential"
                    | "tostringradix"
                    | "rnwith"
                    | "rswith"
                    | "rbwith"
                    | "dynarraywith"
                    | "charat"
                    | "at"
                    | "concat"
                    | "numstr"
                    | "boolstr"
                    | "padend"
                    | "padstart"
                    | "split"
                    | "strarray"
                    | "fromcharcode"
                    | "replace"
                    | "replaceall"
                    | "slice"
                    | "slice2"
                    | "substring"
                    | "substring2"
                    | "tolowercase"
                    | "towellformed"
                    | "touppercase"
                    | "trim"
                    | "trimend"
                    | "trimstart"
                    | "rnjoin"
                    | "rbjoin"
                    | "rsjoin"
                    | "arrayslice"
                    | "arrayconcat"
                    | "arrayreversed"
                    | "arrayreverse"
                    | "rnsorted"
                    | "rssorted"
                    | "rbsorted"
                    | "rnsort"
                    | "rnsortedasc"
                    | "rnsorteddesc"
                    | "rnsortasc"
                    | "rnsortdesc"
                    | "rssorteddesc"
                    | "rssortdesc"
                    | "rssort"
                    | "rbsort"
                    | "rnfill"
                    | "rsfill"
                    | "rbfill"
                    | "arraycopywithin"
                    | "arraysplice"
                    | "arraytospliced"
                    | "dynarraysplice"
                    | "dynarraytospliced"
                    | "dynarrayappend"
                    | "dynarraysometruthy"
                    | "dynarrayeverytruthy"
                    | "dynarrayfindtruthy"
                    | "dynarrayfindindextruthy"
                    | "dynarrayfindlasttruthy"
                    | "dynarrayfindlastindextruthy"
                    | "dynarrayfiltertruthy"
                    | "dynarraymaptonumber"
                    | "dynarraymaptoboolean"
                    | "dynarraymaptostring"
                    | "dynarraymapidentity"
                    | "rnpush"
                    | "rspush"
                    | "rbpush"
                    | "dynarraypush"
                    | "rnunshift"
                    | "rsunshift"
                    | "rbunshift"
                    | "dynarrayunshift"
                    | "rnset"
                    | "rnpostset"
                    | "rsset"
                    | "rbset"
                    | "dynarrayset"
                    | "rnpop"
                    | "rspop"
                    | "rbpop"
                    | "rnshift"
                    | "rsshift"
                    | "rbshift"
                    | "dynarraypop"
                    | "dynarrayshift"
                    | "dkeys"
                    | "dnvalues"
                    | "dbvalues"
                    | "dsvalues"
                    | "dnentries"
                    | "dbentries"
                    | "dsentries"
                    | "dnfromentries"
                    | "dbfromentries"
                    | "dsfromentries"
                    | "missingcalln"
                    | "missingcallb"
                    | "missingcalls"
                    | "missingcalla"
                    | "missingcalld"
                    | "globalget"
                    | "globalinit"
                    | "globalset"
                    | "callableget"
                    | "callableset"
            )
                || token.starts_with("rnreduce")
                || token.starts_with("rnreduceright")
                || token.starts_with("rnfilter")
                || token.starts_with("dynarray")
                || token.contains("mapjit")
                || token.contains("filterjit")
        })
    }

    fn insert_jit_error_checks(output: &mut Vec<String>, start: usize) {
        if !jit_tokens_may_error(&output[start..]) {
            return;
        }
        let tokens = output.split_off(start);
        for token in tokens {
            let may_error = jit_tokens_may_error(std::slice::from_ref(&token));
            output.push(token);
            if may_error {
                output.push("checkerror".into());
            }
        }
    }

    fn split_numeric_body(statements: &[Stmt]) -> Option<(Vec<LocalStep<'_>>, NumericBody<'_>)> {
        let mut steps = Vec::new();
        let mut offset = 0;
        loop {
            match statements.get(offset) {
                Some(Stmt::Decl(Decl::Var(declaration))) => {
                    for declarator in &declaration.decls {
                        let Pat::Ident(name) = &declarator.name else {
                            return None;
                        };
                        steps.push(LocalStep::Declare {
                            name: &name.id,
                            initializer: declarator.init.as_deref()?,
                            mutable: declaration.kind != VarDeclKind::Const,
                        });
                    }
                }
                Some(Stmt::Expr(statement)) => match statement.expr.as_ref() {
                    Expr::Assign(assignment) => {
                        if let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) =
                            &assignment.left
                        {
                            steps.push(LocalStep::Assign {
                                name: &name.id,
                                operation: assignment.op,
                                value: assignment.right.as_ref(),
                            });
                        } else {
                            steps.push(LocalStep::Effect(statement.expr.as_ref()));
                        }
                    }
                    Expr::Update(update) => {
                        if let Expr::Ident(name) = update.arg.as_ref() {
                            steps.push(LocalStep::Update {
                                name,
                                operation: update.op,
                            });
                        } else {
                            steps.push(LocalStep::Effect(statement.expr.as_ref()));
                        }
                    }
                    _ => steps.push(LocalStep::Effect(statement.expr.as_ref())),
                },
                _ => break,
            }
            offset += 1;
        }
        (!statements[offset..].is_empty())
            .then_some((steps, NumericBody::Statements(&statements[offset..])))
    }

    fn encode_for_in_loop(
        statement: &thaw_parser::ast::ForInStmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        state: (
            &mut std::collections::HashMap<String, JitKind>,
            LoopControl,
            &[&thaw_parser::ast::BlockStmt],
        ),
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (kinds, loop_control, finalizers) = state;
        let mut source = Vec::new();
        encode_expression(
            statement.right.as_ref(),
            parameters,
            locals,
            context,
            &mut source,
        )?;
        if jit_expression_kind(&source)?.0 != JitKind::Dictionary {
            return None;
        }
        let source_prefix = dictionary_prefix(&source)?;
        output.extend(source);
        let source_index = kinds.len();
        kinds.insert(
            format!("\0forin-source-{source_index}"),
            JitKind::Dictionary,
        );
        let source_local = format!("{source_prefix}l{source_index}");

        let index = kinds.len();
        output.push(format!("c{:016x}", 0.0f64.to_bits()));
        kinds.insert(format!("\0forin-index-{index}"), JitKind::Number);
        let index_local = format!("ln{index}");

        let element_index = match &statement.left {
            ForHead::VarDecl(declaration) => {
                let [declarator] = declaration.decls.as_slice() else {
                    return None;
                };
                let Pat::Ident(name) = &declarator.name else {
                    return None;
                };
                if declarator.init.is_some()
                    || parameters.contains_key(name.id.sym.as_ref())
                    || locals.contains_key(name.id.sym.as_ref())
                {
                    return None;
                }
                if declaration.kind == VarDeclKind::Const {
                    locals.insert(
                        name.id.sym.to_string(),
                        vec![source_local.clone(), index_local.clone(), "dkeyat".into()],
                    );
                    None
                } else {
                    encode_string("", output)?;
                    let element_index = kinds.len();
                    locals.insert(
                        name.id.sym.to_string(),
                        vec![format!("ls{element_index}")],
                    );
                    kinds.insert(name.id.sym.to_string(), JitKind::String);
                    mutable.insert(name.id.sym.to_string());
                    Some(element_index)
                }
            }
            ForHead::Pat(pattern) => {
                let Pat::Ident(name) = pattern.as_ref() else {
                    return None;
                };
                if !mutable.contains(name.id.sym.as_ref())
                    || kinds.get(name.id.sym.as_ref())? != &JitKind::String
                {
                    return None;
                }
                Some(loop_local_index(
                    locals.get(name.id.sym.as_ref())?.first()?,
                )?)
            }
            ForHead::UsingDecl(_) => return None,
        };

        output.extend([
            "loop".into(),
            index_local.clone(),
            source_local.clone(),
            "dlen".into(),
            "<".into(),
            "while".into(),
        ]);
        if let Some(element_index) = element_index {
            output.extend([
                source_local,
                index_local.clone(),
                "dkeyat".into(),
                format!("setl{element_index}"),
            ]);
        }
        encode_loop_effects(
            statement.body.as_ref(),
            parameters,
            locals,
            mutable,
            (
                kinds,
                nested_loop_control(finalizers.len(), loop_control),
                finalizers,
            ),
            context,
            output,
        )?;
        output.extend([
            "looptail".into(),
            index_local,
            format!("c{:016x}", 1.0f64.to_bits()),
            "+".into(),
            format!("setl{index}"),
            "loopend".into(),
        ]);
        Some(())
    }

    fn encode_loop_switch(
        statement: &thaw_parser::ast::SwitchStmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        control: (
            &std::collections::HashMap<String, JitKind>,
            LoopControl,
            &[&thaw_parser::ast::BlockStmt],
        ),
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (kinds, loop_control, finalizers) = control;
        let mut discriminant = Vec::new();
        encode_expression(
            statement.discriminant.as_ref(),
            parameters,
            locals,
            context,
            &mut discriminant,
        )?;
        let kind = jit_expression_kind(&discriminant)?.0;
        if matches!(kind, JitKind::Array | JitKind::Dictionary) {
            return None;
        }
        output.extend(discriminant);
        output.push("switch".into());
        for case in &statement.cases {
            if let Some(test) = case.test.as_deref() {
                output.extend(["case".into(), "dup".into()]);
                let mut encoded = Vec::new();
                encode_expression(test, parameters, locals, context, &mut encoded)?;
                let test_kind = if boolean_literal(test) {
                    encoded.push("asbool".into());
                    JitKind::Boolean
                } else {
                    jit_expression_kind(&encoded)?.0
                };
                output.extend(encoded);
                if kind == JitKind::String && test_kind == JitKind::String {
                    output.extend([
                        "strcmp".into(),
                        format!("c{:016x}", 0.0f64.to_bits()),
                        "==".into(),
                    ]);
                } else if kind == test_kind
                    && matches!(kind, JitKind::Number | JitKind::Boolean)
                {
                    output.push("==".into());
                } else {
                    output.push("strictfalse".into());
                }
                output.push("casebody".into());
            } else {
                output.push("default".into());
            }
            for statement in &case.cons {
                encode_loop_effects(
                    statement,
                    parameters,
                    locals,
                    mutable,
                    (
                        kinds,
                        LoopControl {
                            break_target: BreakTarget::Switch,
                            break_finalizer_depth: finalizers.len(),
                            continue_finalizer_depth: loop_control.continue_finalizer_depth,
                            throw_finalizer_depth: loop_control.throw_finalizer_depth,
                            catch_active: loop_control.catch_active,
                            catch_tagged: loop_control.catch_tagged,
                        },
                        finalizers,
                    ),
                    context,
                    output,
                )?;
            }
        }
        output.push("switchend".into());
        Some(())
    }

    #[derive(Clone, PartialEq, Eq)]
    struct CaughtThrowKind {
        kind: JitKind,
        prefix: &'static str,
    }

    #[derive(Clone)]
    enum CatchProbe {
        Tag,
        ArrayIndex(u32),
        DictionaryKey(String),
    }

    fn caught_throw_kind(encoded: &[String]) -> Option<CaughtThrowKind> {
        let kind = jit_expression_kind(encoded)?.0;
        let prefix = match kind {
            JitKind::Number => "ln",
            JitKind::Boolean => "lb",
            JitKind::String => "ls",
            JitKind::Dynamic => "ld",
            JitKind::Array => match array_prefix(encoded)? {
                "rn" => "rnl",
                "rb" => "rbl",
                "rs" => "rsl",
                _ => return None,
            },
            JitKind::Dictionary => match dictionary_prefix(encoded)? {
                "dn" => "dnl",
                "db" => "dbl",
                "ds" => "dsl",
                _ => return None,
            },
        };
        Some(CaughtThrowKind { kind, prefix })
    }

    fn caught_throw_tag(kind: &CaughtThrowKind) -> Option<u8> {
        match kind.prefix {
            "ln" => Some(0),
            "lb" => Some(1),
            "ls" => Some(2),
            "rnl" => Some(3),
            "rbl" => Some(4),
            "rsl" => Some(5),
            "dnl" => Some(6),
            "dbl" => Some(7),
            "dsl" => Some(8),
            _ => None,
        }
    }

    fn collect_caught_native_error(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut Vec<CaughtThrowKind>,
    ) -> Option<()> {
        let mut encoded = Vec::new();
        if encode_expression(expression, parameters, locals, context, &mut encoded).is_none() {
            return Some(());
        }
        let error = CaughtThrowKind {
            kind: JitKind::String,
            prefix: "ls",
        };
        if jit_tokens_may_error(&encoded) && !kinds.contains(&error) {
            kinds.push(error);
        }
        Some(())
    }

    fn collect_caught_throw_kind(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kinds: &mut Vec<CaughtThrowKind>,
    ) -> Option<()> {
        match statement {
            Stmt::Throw(thrown) => {
                let mut encoded = Vec::new();
                encode_expression(
                    thrown.arg.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                let error = CaughtThrowKind {
                    kind: JitKind::String,
                    prefix: "ls",
                };
                if jit_tokens_may_error(&encoded) && !kinds.contains(&error) {
                    kinds.push(error);
                }
                let thrown_kind = caught_throw_kind(&encoded)?;
                if !kinds.contains(&thrown_kind) {
                    kinds.push(thrown_kind);
                }
            }
            Stmt::Expr(statement) => collect_caught_native_error(
                statement.expr.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::Return(statement) => {
                if let Some(expression) = statement.arg.as_deref() {
                    collect_caught_native_error(
                        expression, parameters, locals, context, kinds,
                    )?;
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                }
            }
            Stmt::If(branch) => {
                collect_caught_native_error(
                    branch.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_throw_kind(
                    branch.cons.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    collect_caught_throw_kind(
                        alternate,
                        parameters,
                        locals,
                        context,
                        kinds,
                    )?;
                }
            }
            Stmt::While(statement) => {
                collect_caught_native_error(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_throw_kind(
                    statement.body.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
            }
            Stmt::DoWhile(statement) => {
                collect_caught_throw_kind(
                    statement.body.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
                collect_caught_native_error(
                    statement.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    kinds,
                )?;
            }
            Stmt::For(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::ForIn(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::ForOf(statement) => collect_caught_throw_kind(
                statement.body.as_ref(),
                parameters,
                locals,
                context,
                kinds,
            )?,
            Stmt::Switch(statement) => {
                for statement in statement.cases.iter().flat_map(|case| &case.cons) {
                    collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                }
            }
            Stmt::Try(statement) => {
                if let Some(handler) = &statement.handler {
                    let mut nested_kinds = Vec::new();
                    for statement in &statement.block.stmts {
                        collect_caught_throw_kind(
                            statement,
                            parameters,
                            locals,
                            context,
                            &mut nested_kinds,
                        )?;
                    }
                    let mut handler_locals = locals.clone();
                    if let (Some(Pat::Ident(identifier)), [nested_kind]) =
                        (&handler.param, nested_kinds.as_slice())
                    {
                        handler_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("{}0", nested_kind.prefix)],
                        );
                    }
                    for statement in &handler.body.stmts {
                        collect_caught_throw_kind(
                            statement,
                            parameters,
                            &handler_locals,
                            context,
                            kinds,
                        )?;
                    }
                } else {
                    for statement in &statement.block.stmts {
                        collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_caught_throw_kind(statement, parameters, locals, context, kinds)?;
                    }
                }
            }
            _ => {}
        }
        Some(())
    }

    fn dynamic_catch_narrowing(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(
        String,
        usize,
        usize,
        CaughtThrowKind,
        Option<CaughtThrowKind>,
        CatchProbe,
    )> {
        let typeof_target = |expression: &Expr| {
            let Expr::Unary(unary) = expression else {
                return None;
            };
            if unary.op != UnaryOp::TypeOf {
                return None;
            }
            match unary.arg.as_ref() {
                Expr::Ident(identifier) => Some((identifier.sym.to_string(), CatchProbe::Tag)),
                Expr::Member(member) => {
                    let Expr::Ident(identifier) = member.obj.as_ref() else {
                        return None;
                    };
                    let probe = match &member.prop {
                        MemberProp::Ident(property) => {
                            CatchProbe::DictionaryKey(property.sym.to_string())
                        }
                        MemberProp::Computed(property) => match property.expr.as_ref() {
                            Expr::Lit(Lit::Num(index))
                                if index.value >= 0.0
                                    && index.value.fract() == 0.0
                                    && index.value <= u32::MAX as f64 =>
                            {
                                CatchProbe::ArrayIndex(index.value as u32)
                            }
                            Expr::Lit(Lit::Str(key)) => CatchProbe::DictionaryKey(
                                key.value.to_string_lossy().into_owned(),
                            ),
                            _ => return None,
                        },
                        _ => return None,
                    };
                    Some((identifier.sym.to_string(), probe))
                }
                _ => None,
            }
        };
        let literal = |expression: &Expr| {
            let Expr::Lit(Lit::Str(value)) = expression else {
                return None;
            };
            Some(value.value.to_string_lossy().into_owned())
        };
        let (name, selector, probe) = if let Expr::Bin(binary) = expression {
            if !matches!(binary.op, BinaryOp::EqEq | BinaryOp::EqEqEq) {
                return None;
            }
            let ((name, probe), type_name) = typeof_target(binary.left.as_ref())
                .zip(literal(binary.right.as_ref()))
                .or_else(|| {
                    literal(binary.left.as_ref())
                        .zip(typeof_target(binary.right.as_ref()))
                        .map(|(literal, target)| (target, literal))
                })?;
            let base = match &probe {
                CatchProbe::Tag => 0,
                CatchProbe::ArrayIndex(_) => 3,
                CatchProbe::DictionaryKey(_) => 6,
            };
            let selector = match type_name.as_str() {
                "number" => Some(base),
                "boolean" => Some(base + 1),
                "string" => Some(base + 2),
                "object" => return None,
                _ => return None,
            };
            (name, selector, probe)
        } else {
            let Expr::Call(call) = expression else {
                return None;
            };
            let Callee::Expr(callee) = &call.callee else {
                return None;
            };
            let Expr::Member(member) = callee.as_ref() else {
                return None;
            };
            let (Expr::Ident(object), MemberProp::Ident(property)) =
                (member.obj.as_ref(), &member.prop)
            else {
                return None;
            };
            if object.sym != *"Array" || property.sym != *"isArray" {
                return None;
            }
            let [argument] = call.args.as_slice() else {
                return None;
            };
            if argument.spread.is_some() {
                return None;
            }
            let Expr::Ident(identifier) = argument.expr.as_ref() else {
                return None;
            };
            (identifier.sym.to_string(), None, CatchProbe::Tag)
        };
        let [marker] = locals.get(&name)?.as_slice() else {
            return None;
        };
        let marker = marker.strip_prefix('x')?;
        let mut fields = marker.splitn(3, ':');
        let tag_index = fields.next()?.parse().ok()?;
        let value_index = fields.next()?.parse().ok()?;
        let variants = fields
            .next()?
            .split('|')
            .map(|variant| {
                let (tag, prefix) = variant.split_once('=')?;
                let tag = tag.parse::<u8>().ok()?;
                let (kind, prefix) = match prefix {
                    "ln" => (JitKind::Number, "ln"),
                    "lb" => (JitKind::Boolean, "lb"),
                    "ls" => (JitKind::String, "ls"),
                    "rnl" => (JitKind::Array, "rnl"),
                    "rbl" => (JitKind::Array, "rbl"),
                    "rsl" => (JitKind::Array, "rsl"),
                    "dnl" => (JitKind::Dictionary, "dnl"),
                    "dbl" => (JitKind::Dictionary, "dbl"),
                    "dsl" => (JitKind::Dictionary, "dsl"),
                    _ => return None,
                };
                Some((tag, CaughtThrowKind { kind, prefix }))
            })
            .collect::<Option<Vec<_>>>()?;
        let selected = if let Some(expected_tag) = selector {
            variants
                .iter()
                .find(|(tag, _)| *tag == expected_tag)?
                .1
                .clone()
        } else {
            let mut objects = variants.iter().filter(|(_, kind)| kind.kind == JitKind::Array);
            let selected = objects.next()?.1.clone();
            if objects.next().is_some() {
                return None;
            }
            selected
        };
        let selected_tag = caught_throw_tag(&selected)?;
        let alternate = (matches!(&probe, CatchProbe::Tag) && variants.len() == 2)
            .then(|| {
                variants
                    .iter()
                    .find(|(tag, _)| *tag != selected_tag)
                    .map(|(_, kind)| kind.clone())
            })
            .flatten();
        Some((name, tag_index, value_index, selected, alternate, probe))
    }

    fn encode_loop_effects(
        statement: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        control: (
            &std::collections::HashMap<String, JitKind>,
            LoopControl,
            &[&thaw_parser::ast::BlockStmt],
        ),
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (kinds, loop_control, finalizers) = control;

        fn encode_finalizers(
            finalizers: &[&thaw_parser::ast::BlockStmt],
            from: usize,
            scope: LoopScope<'_>,
            loop_control: LoopControl,
            context: &mut InlineContext<'_>,
            output: &mut Vec<String>,
        ) -> Option<()> {
            let (parameters, locals, mutable, kinds) = scope;
            for index in (from..finalizers.len()).rev() {
                encode_loop_effects(
                    &Stmt::Block(finalizers[index].clone()),
                    parameters,
                    locals,
                    mutable,
                    (kinds, loop_control, &finalizers[..index]),
                    context,
                    output,
                )?;
            }
            Some(())
        }
        if let Stmt::Block(block) = statement {
            let mut block_locals = locals.clone();
            let mut block_mutable = mutable.clone();
            let mut block_kinds = kinds.clone();
            let mut declared = std::collections::HashSet::new();
            for statement in &block.stmts {
                if let Stmt::Decl(Decl::Var(declaration)) = statement {
                    for declarator in &declaration.decls {
                        let Pat::Ident(name) = &declarator.name else {
                            return None;
                        };
                        if !declared.insert(name.id.sym.to_string()) {
                            return None;
                        }
                        block_locals.remove(name.id.sym.as_ref());
                        block_mutable.remove(name.id.sym.as_ref());
                        if let Some(kind) = block_kinds.remove(name.id.sym.as_ref()) {
                            let slot = block_kinds.len();
                            block_kinds.insert(format!("\0shadow-{slot}"), kind);
                        }
                        let start = output.len();
                        encode_loop_declaration(
                            LocalStep::Declare {
                                name: &name.id,
                                initializer: declarator.init.as_deref()?,
                                mutable: declaration.kind != VarDeclKind::Const,
                            },
                            None,
                            parameters,
                            &mut block_locals,
                            &mut block_mutable,
                            &mut block_kinds,
                            context,
                            output,
                        )?;
                        if loop_control.catch_active {
                            insert_jit_error_checks(output, start);
                        }
                    }
                    continue;
                }
                encode_loop_effects(
                    statement,
                    parameters,
                    &block_locals,
                    &block_mutable,
                    (&block_kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
            }
            output.extend(std::iter::repeat_n(
                "drop".into(),
                block_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::If(branch) = statement {
            if let Some((name, tag_index, value_index, selected, alternate_kind, probe)) =
                dynamic_catch_narrowing(branch.test.as_ref(), locals)
            {
                output.extend([
                    format!("ln{tag_index}"),
                    format!("c{:016x}", f64::from(caught_throw_tag(&selected)?).to_bits()),
                    "==".into(),
                ]);
                match probe {
                    CatchProbe::Tag => {}
                    CatchProbe::ArrayIndex(index) => output.extend([
                        "if".into(),
                        format!("{}{value_index}", selected.prefix),
                        "arraylen".into(),
                        format!("c{:016x}", f64::from(index).to_bits()),
                        ">".into(),
                        "else".into(),
                        "c0000000000000000".into(),
                        "end".into(),
                    ]),
                    CatchProbe::DictionaryKey(key) => {
                        output.push("if".into());
                        encode_string(&key, output)?;
                        output.extend([
                            format!("{}{value_index}", selected.prefix),
                            "din".into(),
                            "else".into(),
                            "c0000000000000000".into(),
                            "end".into(),
                        ]);
                    }
                }
                output.push("guard".into());
                let mut selected_locals = locals.clone();
                selected_locals.insert(
                    name.clone(),
                    vec![format!("{}{value_index}", selected.prefix)],
                );
                let mut selected_kinds = kinds.clone();
                selected_kinds.insert(name.clone(), selected.kind);
                encode_loop_effects(
                    branch.cons.as_ref(),
                    parameters,
                    &selected_locals,
                    mutable,
                    (&selected_kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    let alternate_kind = alternate_kind?;
                    output.push("guardelse".into());
                    let mut alternate_locals = locals.clone();
                    alternate_locals.insert(
                        name.clone(),
                        vec![format!("{}{value_index}", alternate_kind.prefix)],
                    );
                    let mut alternate_kinds = kinds.clone();
                    alternate_kinds.insert(name, alternate_kind.kind);
                    encode_loop_effects(
                        alternate,
                        parameters,
                        &alternate_locals,
                        mutable,
                        (&alternate_kinds, loop_control, finalizers),
                        context,
                        output,
                    )?;
                }
                output.push("guardend".into());
                return Some(());
            }
            let start = output.len();
            encode_condition(branch.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            if let Some((consequent_locals, alternate_locals)) = narrowed_locals(
                branch.test.as_ref(),
                parameters,
                locals,
                context.helpers,
            )
            {
                output.push("guard".into());
                encode_loop_effects(
                    branch.cons.as_ref(),
                    parameters,
                    &consequent_locals,
                    mutable,
                    (kinds, loop_control, finalizers),
                    context,
                    output,
                )?;
                if let Some(alternate) = branch.alt.as_deref() {
                    output.push("guardelse".into());
                    encode_loop_effects(
                        alternate,
                        parameters,
                        &alternate_locals,
                        mutable,
                        (kinds, loop_control, finalizers),
                        context,
                        output,
                    )?;
                }
                output.push("guardend".into());
                return Some(());
            }
            output.push("guard".into());
            encode_loop_effects(
                branch.cons.as_ref(), parameters, locals, mutable, (kinds, loop_control, finalizers), context, output,
            )?;
            if let Some(alternate) = branch.alt.as_deref() {
                output.push("guardelse".into());
                encode_loop_effects(
                    alternate, parameters, locals, mutable, (kinds, loop_control, finalizers), context, output,
                )?;
            }
            output.push("guardend".into());
            return Some(());
        }
        if matches!(statement, Stmt::Break(statement) if statement.label.is_none()) {
            encode_finalizers(
                finalizers,
                loop_control.break_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(match loop_control.break_target {
                BreakTarget::Loop => "break",
                BreakTarget::Switch => "switchbreak",
            }.into());
            return Some(());
        }
        if matches!(statement, Stmt::Continue(statement) if statement.label.is_none()) {
            encode_finalizers(
                finalizers,
                loop_control.continue_finalizer_depth,
                (parameters, locals, mutable, kinds),
                loop_control,
                context,
                output,
            )?;
            output.push("continue".into());
            return Some(());
        }
        if let Stmt::Switch(statement) = statement {
            return encode_loop_switch(
                statement,
                parameters,
                locals,
                mutable,
                (kinds, loop_control, finalizers),
                context,
                output,
            );
        }
        if let Stmt::Try(statement) = statement {
            let mut nested_finalizers = finalizers.to_vec();
            if let Some(finalizer) = &statement.finalizer {
                nested_finalizers.push(finalizer);
            }
            if let Some(handler) = &statement.handler {
                let mut caught_kinds = Vec::new();
                for statement in &statement.block.stmts {
                    collect_caught_throw_kind(
                        statement,
                        parameters,
                        locals,
                        context,
                        &mut caught_kinds,
                    )?;
                }
                if caught_kinds.is_empty() {
                    caught_kinds.push(CaughtThrowKind {
                        kind: JitKind::String,
                        prefix: "ls",
                    });
                }
                let tagged = caught_kinds.len() > 1;
                output.push(if tagged { "trystarttag" } else { "trystart" }.into());
                let caught_control = LoopControl {
                    throw_finalizer_depth: nested_finalizers.len(),
                    catch_active: true,
                    catch_tagged: tagged,
                    ..loop_control
                };
                encode_loop_effects(
                    &Stmt::Block(statement.block.clone()),
                    parameters,
                    locals,
                    mutable,
                    (kinds, caught_control, &nested_finalizers),
                    context,
                    output,
                )?;
                output.push("catch".into());

                let mut catch_locals = locals.clone();
                let mut handler_kinds = kinds.clone();
                if let Some(parameter) = &handler.param {
                    let Pat::Ident(identifier) = parameter else {
                        return None;
                    };
                    if parameters.contains_key(identifier.id.sym.as_ref())
                        || locals.contains_key(identifier.id.sym.as_ref())
                    {
                        return None;
                    }
                    let index = handler_kinds.len();
                    if tagged {
                        let variants = caught_kinds
                            .iter()
                            .map(|kind| {
                                Some(format!(
                                    "{}={}",
                                    caught_throw_tag(kind)?,
                                    kind.prefix
                                ))
                            })
                            .collect::<Option<Vec<_>>>()?
                            .join("|");
                        catch_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("x{index}:{}:{variants}", index + 1)],
                        );
                        handler_kinds.insert(
                            format!("\0catch-tag-{index}"),
                            JitKind::Number,
                        );
                        handler_kinds.insert(identifier.id.sym.to_string(), JitKind::Number);
                    } else {
                        let caught_kind = &caught_kinds[0];
                        catch_locals.insert(
                            identifier.id.sym.to_string(),
                            vec![format!("{}{index}", caught_kind.prefix)],
                        );
                        handler_kinds.insert(identifier.id.sym.to_string(), caught_kind.kind);
                    }
                } else {
                    let slots = if tagged { 2 } else { 1 };
                    for slot in 0..slots {
                        handler_kinds.insert(
                            format!("\0catch-{}-{slot}", handler_kinds.len()),
                            JitKind::Number,
                        );
                    }
                }
                encode_loop_effects(
                    &Stmt::Block(handler.body.clone()),
                    parameters,
                    &catch_locals,
                    mutable,
                    (&handler_kinds, loop_control, &nested_finalizers),
                    context,
                    output,
                )?;
                output.extend(std::iter::repeat_n("drop".into(), if tagged { 2 } else { 1 }));
                output.push("tryend".into());
                if let Some(finalizer) = &statement.finalizer {
                    return encode_loop_effects(
                        &Stmt::Block(finalizer.clone()),
                        parameters,
                        locals,
                        mutable,
                        (kinds, loop_control, finalizers),
                        context,
                        output,
                    );
                }
                return Some(());
            }
            let finalizer = statement.finalizer.as_ref()?;
            encode_loop_effects(
                &Stmt::Block(statement.block.clone()),
                parameters,
                locals,
                mutable,
                (kinds, loop_control, &nested_finalizers),
                context,
                output,
            )?;
            return encode_loop_effects(
                &Stmt::Block(finalizer.clone()),
                parameters,
                locals,
                mutable,
                (kinds, loop_control, finalizers),
                context,
                output,
            );
        }
        if let Stmt::Throw(thrown) = statement {
            let mut encoded = Vec::new();
            encode_expression(
                thrown.arg.as_ref(),
                parameters,
                locals,
                context,
                &mut encoded,
            )?;
            let caught_kind = caught_throw_kind(&encoded)?;
            let kind = caught_kind.kind;
            let may_error = jit_tokens_may_error(&encoded);
            let start = output.len();
            output.extend(encoded);
            if loop_control.catch_active && may_error {
                insert_jit_error_checks(output, start);
            }
            let mut throw_kinds = kinds.clone();
            throw_kinds.insert(format!("\0throw-{}", throw_kinds.len()), kind);
            encode_finalizers(
                finalizers,
                loop_control.throw_finalizer_depth,
                (parameters, locals, mutable, &throw_kinds),
                loop_control,
                context,
                output,
            )?;
            output.push(
                if loop_control.catch_active {
                    if loop_control.catch_tagged {
                        output.push(format!("throwtag{}", caught_throw_tag(&caught_kind)?));
                        return Some(());
                    }
                    "throw"
                } else {
                    match caught_kind.prefix {
                        "rnl" => "throwoutrn",
                        "rbl" => "throwoutrb",
                        "rsl" => "throwoutrs",
                        "dnl" | "dbl" | "dsl" => "throwoutd",
                        _ => match kind {
                        JitKind::Number => "throwoutn",
                        JitKind::Boolean => "throwoutb",
                        JitKind::String => "throwouts",
                        JitKind::Dynamic => return None,
                        JitKind::Array | JitKind::Dictionary => return None,
                        },
                    }
                }
                .into(),
            );
            return Some(());
        }
        if let Stmt::Return(returned) = statement {
            let mut encoded = Vec::new();
            encode_expression(
                returned.arg.as_deref()?, parameters, locals, context, &mut encoded,
            )?;
            let kind = jit_expression_kind(&encoded).map(|(kind, _)| kind);
            if kind == Some(JitKind::Array) {
                encoded.push("arrayvalue".into());
            }
            let may_error = jit_tokens_may_error(&encoded);
            let start = output.len();
            output.extend(encoded);
            if loop_control.catch_active && may_error {
                insert_jit_error_checks(output, start);
            }
            let mut return_kinds = kinds.clone();
            return_kinds.insert(
                format!("\0return-{}", return_kinds.len()),
                kind.unwrap_or(JitKind::Number),
            );
            encode_finalizers(
                finalizers,
                0,
                (parameters, locals, mutable, &return_kinds),
                loop_control,
                context,
                output,
            )?;
            output.push("return".into());
            return Some(());
        }
        if let Stmt::While(loop_statement) = statement {
            output.extend(["loop".into(), "looptail".into()]);
            let start = output.len();
            encode_condition(loop_statement.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            output.push("while".into());
            encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                locals,
                mutable,
                (
                    kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            )?;
            output.push("loopend".into());
            return Some(());
        }
        if let Stmt::DoWhile(loop_statement) = statement {
            output.push("loop".into());
            encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                locals,
                mutable,
                (
                    kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            )?;
            output.push("looptail".into());
            let start = output.len();
            encode_condition(loop_statement.test.as_ref(), parameters, locals, context, output)?;
            if loop_control.catch_active {
                insert_jit_error_checks(output, start);
            }
            output.extend(["while".into(), "loopend".into()]);
            return Some(());
        }
        if let Stmt::For(loop_statement) = statement {
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            match loop_statement.init.as_ref() {
                Some(VarDeclOrExpr::Expr(initializer)) => encode_loop_expression(
                    initializer.as_ref(),
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_kinds,
                    context,
                    output,
                )?,
                None => {}
                Some(VarDeclOrExpr::VarDecl(declaration)) => {
                    for declarator in &declaration.decls {
                        let Pat::Ident(name) = &declarator.name else {
                            return None;
                        };
                        encode_loop_declaration(
                            LocalStep::Declare {
                                name: &name.id,
                                initializer: declarator.init.as_deref()?,
                                mutable: declaration.kind != VarDeclKind::Const,
                            },
                            None,
                            parameters,
                            &mut nested_locals,
                            &mut nested_mutable,
                            &mut nested_kinds,
                            context,
                            output,
                        )?;
                    }
                }
            }
            output.push("loop".into());
            if let Some(test) = loop_statement.test.as_deref() {
                let start = output.len();
                encode_condition(test, parameters, &nested_locals, context, output)?;
                if loop_control.catch_active {
                    insert_jit_error_checks(output, start);
                }
            } else {
                output.extend([
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "asbool".into(),
                ]);
            }
            output.push("while".into());
            encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                &nested_locals,
                &nested_mutable,
                (
                    &nested_kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            )?;
            output.push("looptail".into());
            if let Some(update) = loop_statement.update.as_deref() {
                let start = output.len();
                encode_loop_expression(
                    update,
                    parameters,
                    &nested_locals,
                    &nested_mutable,
                    &nested_kinds,
                    context,
                    output,
                )?;
                if loop_control.catch_active {
                    insert_jit_error_checks(output, start);
                }
            }
            output.push("loopend".into());
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::ForOf(loop_statement) = statement {
            if loop_statement.is_await {
                return None;
            }
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            let mut source = Vec::new();
            encode_expression(
                loop_statement.right.as_ref(),
                parameters,
                &nested_locals,
                context,
                &mut source,
            )?;
            if jit_expression_kind(&source)?.0 != JitKind::Array {
                return None;
            }
            let array = array_prefix(&source)?;
            let element_kind = match array {
                "rn" => JitKind::Number,
                "rb" => JitKind::Boolean,
                "rs" => JitKind::String,
                _ => return None,
            };
            let source_index = nested_kinds.len();
            output.extend(source);
            nested_kinds.insert(format!("\0forof-source-{source_index}"), JitKind::Array);
            let source_local = format!("{array}l{source_index}");
            let index = nested_kinds.len();
            output.push(format!("c{:016x}", 0.0f64.to_bits()));
            nested_kinds.insert(format!("\0forof-index-{index}"), JitKind::Number);
            let index_local = format!("ln{index}");
            let element_index = match &loop_statement.left {
                ForHead::VarDecl(declaration) => {
                    let [declarator] = declaration.decls.as_slice() else {
                        return None;
                    };
                    let Pat::Ident(name) = &declarator.name else {
                        return None;
                    };
                    if declarator.init.is_some()
                        || parameters.contains_key(name.id.sym.as_ref())
                        || nested_locals.contains_key(name.id.sym.as_ref())
                    {
                        return None;
                    }
                    match element_kind {
                        JitKind::String => encode_string("", output)?,
                        JitKind::Number | JitKind::Boolean => {
                            output.push(format!("c{:016x}", 0.0f64.to_bits()));
                            if element_kind == JitKind::Boolean {
                                output.push("asbool".into());
                            }
                        }
                        _ => return None,
                    }
                    let element_index = nested_kinds.len();
                    let prefix = match element_kind {
                        JitKind::Number => "ln",
                        JitKind::Boolean => "lb",
                        JitKind::String => "ls",
                        _ => return None,
                    };
                    nested_locals.insert(
                        name.id.sym.to_string(),
                        vec![format!("{prefix}{element_index}")],
                    );
                    nested_kinds.insert(name.id.sym.to_string(), element_kind);
                    if declaration.kind != VarDeclKind::Const {
                        nested_mutable.insert(name.id.sym.to_string());
                    }
                    element_index
                }
                ForHead::Pat(pattern) => {
                    let Pat::Ident(name) = pattern.as_ref() else {
                        return None;
                    };
                    if !nested_mutable.contains(name.id.sym.as_ref())
                        || nested_kinds.get(name.id.sym.as_ref())? != &element_kind
                    {
                        return None;
                    }
                    nested_locals
                        .get(name.id.sym.as_ref())?
                        .first()?
                        .get(2..)?
                        .parse::<usize>()
                        .ok()?
                }
                ForHead::UsingDecl(_) => return None,
            };
            output.push("loop".into());
            output.extend([
                index_local.clone(),
                source_local.clone(),
                "arraylen".into(),
                "<".into(),
                "while".into(),
                source_local,
                index_local.clone(),
                format!("{array}get"),
                format!("setl{element_index}"),
            ]);
            encode_loop_effects(
                loop_statement.body.as_ref(),
                parameters,
                &nested_locals,
                &nested_mutable,
                (
                    &nested_kinds,
                    nested_loop_control(finalizers.len(), loop_control),
                    finalizers,
                ),
                context,
                output,
            )?;
            output.extend([
                "looptail".into(),
                index_local,
                format!("c{:016x}", 1.0f64.to_bits()),
                "+".into(),
                format!("setl{index}"),
                "loopend".into(),
            ]);
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        if let Stmt::ForIn(loop_statement) = statement {
            let mut nested_locals = locals.clone();
            let mut nested_mutable = mutable.clone();
            let mut nested_kinds = kinds.clone();
            encode_for_in_loop(
                loop_statement,
                parameters,
                &mut nested_locals,
                &mut nested_mutable,
                (&mut nested_kinds, loop_control, finalizers),
                context,
                output,
            )?;
            output.extend(std::iter::repeat_n(
                "drop".into(),
                nested_kinds.len().checked_sub(kinds.len())?,
            ));
            return Some(());
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        let start = output.len();
        encode_loop_expression(
            statement.expr.as_ref(), parameters, locals, mutable, kinds, context, output,
        )?;
        if loop_control.catch_active {
            insert_jit_error_checks(output, start);
        }
        Some(())
    }

    fn encode_loop_expression(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if encode_callable_table_update(expression, parameters, locals, context, output).is_some() {
            return Some(());
        }
        match expression {
            Expr::Assign(assignment) => {
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    encode_expression(expression, parameters, locals, context, output)?;
                    output.push("drop".into());
                    return Some(());
                };
                encode_local_assignment(
                    &name.id,
                    assignment.op,
                    assignment.right.as_ref(),
                    parameters,
                    locals,
                    mutable,
                    kinds,
                    context,
                    output,
                )?;
            }
            Expr::Update(update) => {
                let Expr::Ident(name) = update.arg.as_ref() else {
                    return None;
                };
                encode_local_update(name, update.op, locals, mutable, kinds, output)?;
            }
            expression => {
                encode_expression(expression, parameters, locals, context, output)?;
                output.push("drop".into());
            }
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_local_assignment(
        name: &Ident,
        operation: AssignOp,
        value_expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if !mutable.contains(name.sym.as_ref()) {
            return None;
        }
        let local = locals.get(name.sym.as_ref())?.first()?.clone();
        if let Some((local, helpers)) = locals
            .get(name.sym.as_ref())
            .and_then(|tokens| callable_local_alias(tokens))
        {
            if operation != AssignOp::Assign {
                return None;
            }
            return encode_callable_local_reassignment(
                value_expression,
                &local,
                &helpers,
                parameters,
                locals,
                context,
                output,
            );
        }
        let index = loop_local_index(&local)?;
        if operation != AssignOp::Assign {
            output.push(local.clone());
        }
        let mut value = Vec::new();
        encode_expression(value_expression, parameters, locals, context, &mut value)?;
        match kinds.get(name.sym.as_ref())? {
            JitKind::Boolean => value.push("asbool".into()),
            JitKind::Dynamic => {
                let value_kind = jit_expression_kind(&value)?.0;
                tag_jit_value(&mut value, value_kind)?;
            }
            JitKind::Array if array_prefix(&value)? != local.get(..2)? => return None,
            JitKind::Dictionary if dictionary_prefix(&value)? != local.get(..2)? => return None,
            _ => {}
        }
        output.extend(value);
        if kinds.get(name.sym.as_ref())? == &JitKind::Array && operation == AssignOp::Assign {
            output.push("arrayhandle".into());
        }
        if operation != AssignOp::Assign {
            output.push(
                match (kinds.get(name.sym.as_ref())?, operation) {
                    (JitKind::String, AssignOp::AddAssign) => "concat",
                    (JitKind::Number, AssignOp::AddAssign) => "+",
                    (JitKind::Number, AssignOp::SubAssign) => "-",
                    (JitKind::Number, AssignOp::MulAssign) => "*",
                    (JitKind::Number, AssignOp::DivAssign) => "/",
                    (JitKind::Number, AssignOp::ModAssign) => "%",
                    (JitKind::Number, AssignOp::LShiftAssign) => "shl",
                    (JitKind::Number, AssignOp::RShiftAssign) => "shr",
                    (JitKind::Number, AssignOp::ZeroFillRShiftAssign) => "ushr",
                    (JitKind::Number, AssignOp::BitOrAssign) => "bor",
                    (JitKind::Number, AssignOp::BitXorAssign) => "bxor",
                    (JitKind::Number, AssignOp::BitAndAssign) => "band",
                    (JitKind::Number, AssignOp::ExpAssign) => "pow",
                    _ => return None,
                }
                .into(),
            );
        }
        output.push(format!("setl{index}"));
        Some(())
    }

    fn encode_local_update(
        name: &Ident,
        operation: UpdateOp,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        kinds: &std::collections::HashMap<String, JitKind>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if !mutable.contains(name.sym.as_ref())
            || kinds.get(name.sym.as_ref())? != &JitKind::Number
        {
            return None;
        }
        let local = locals.get(name.sym.as_ref())?.first()?.clone();
        let index = loop_local_index(&local)?;
        output.extend([
            local,
            format!("c{:016x}", 1.0f64.to_bits()),
            match operation {
                UpdateOp::PlusPlus => "+".into(),
                UpdateOp::MinusMinus => "-".into(),
            },
            format!("setl{index}"),
        ]);
        Some(())
    }

    fn loop_local_index(token: &str) -> Option<usize> {
        token.get(token.find(|character: char| character.is_ascii_digit())?..)?
            .parse()
            .ok()
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_loop_declaration(
        step: LocalStep<'_>,
        forced_kind: Option<(JitKind, std::collections::HashSet<JitKind>)>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let LocalStep::Declare {
            name,
            initializer,
            mutable: is_mutable,
        } = step
        else {
            return None;
        };
        if parameters.contains_key(name.sym.as_ref()) || locals.contains_key(name.sym.as_ref()) {
            return None;
        }
        let mut encoded = Vec::new();
        encode_expression(initializer, parameters, locals, context, &mut encoded)?;
        let value_kind = if boolean_literal(initializer) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&encoded)?.0
        };
        let (kind, candidates) = forced_kind.unwrap_or_else(|| {
            let candidates = if value_kind == JitKind::Dynamic {
                encoded
                    .iter()
                    .filter_map(|token| match token.as_str() {
                        "tagnum" => Some(JitKind::Number),
                        "tagbool" => Some(JitKind::Boolean),
                        "tagstr" => Some(JitKind::String),
                        "tagrn" | "tagrb" | "tagrs" => Some(JitKind::Array),
                        "tagdn" | "tagdb" | "tagds" => Some(JitKind::Dictionary),
                        "tagtuple" => Some(JitKind::Array),
                        _ => None,
                    })
                    .collect()
            } else {
                std::iter::once(value_kind).collect()
            };
            (value_kind, candidates)
        });
        if kind == JitKind::Dynamic && value_kind != JitKind::Dynamic {
            tag_jit_value(&mut encoded, value_kind)?;
        } else if kind != value_kind {
            return None;
        }
        let prefix = match kind {
            JitKind::Number => "ln",
            JitKind::Boolean => "lb",
            JitKind::String => "ls",
            JitKind::Dynamic => "ld",
            JitKind::Array => match array_prefix(&encoded)? {
                "rn" => "rnl",
                "rb" => "rbl",
                "rs" => "rsl",
                _ => return None,
            },
            JitKind::Dictionary => match dictionary_prefix(&encoded)? {
                "dn" => "dnl",
                "db" => "dbl",
                "ds" => "dsl",
                _ => return None,
            },
        };
        let index = kinds.len();
        output.extend(encoded);
        if kind == JitKind::Boolean {
            output.push("asbool".into());
        } else if kind == JitKind::Array {
            output.push("arrayhandle".into());
        }
        let mut local = vec![format!("{prefix}{index}")];
        if kind == JitKind::Dynamic {
            for (candidate, marker) in [
                (JitKind::Number, "notnum"),
                (JitKind::Boolean, "notbool"),
                (JitKind::String, "notstr"),
            ] {
                if !candidates.contains(&candidate) {
                    local.push(marker.into());
                }
            }
        }
        locals.insert(name.sym.to_string(), local);
        kinds.insert(name.sym.to_string(), kind);
        if is_mutable {
            mutable.insert(name.sym.to_string());
        }
        Some(())
    }

    fn collect_loop_callable_assignments(
        statement: &Stmt,
        context: &InlineContext<'_>,
        output: &mut std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                let Expr::Assign(assignment) = statement.expr.as_ref() else {
                    return;
                };
                if assignment.op != AssignOp::Assign {
                    return;
                }
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    return;
                };
                if let Some(helpers) = finite_callable_names(assignment.right.as_ref()).or_else(|| {
                    callable_member_helpers(assignment.right.as_ref(), context)
                }) {
                    if helpers
                        .iter()
                        .all(|helper| context.helpers.contains_key(helper))
                    {
                        output
                            .entry(name.id.sym.to_string())
                            .or_default()
                            .extend(helpers);
                    }
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_loop_callable_assignments(statement, context, output);
                }
            }
            Stmt::If(statement) => {
                collect_loop_callable_assignments(statement.cons.as_ref(), context, output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_loop_callable_assignments(alternate, context, output);
                }
            }
            Stmt::While(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::DoWhile(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::For(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::ForOf(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::ForIn(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_loop_callable_assignments(statement, context, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_loop_callable_assignments(statement, context, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_loop_callable_assignments(statement.body.as_ref(), context, output);
            }
            _ => {}
        }
    }

    fn collect_local_value_assignments<'a>(
        statement: &'a Stmt,
        output: &mut std::collections::HashMap<String, Vec<&'a Expr>>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                let Expr::Assign(assignment) = statement.expr.as_ref() else {
                    return;
                };
                if assignment.op != AssignOp::Assign {
                    return;
                }
                let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
                    return;
                };
                output
                    .entry(name.id.sym.to_string())
                    .or_default()
                    .push(assignment.right.as_ref());
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_local_value_assignments(statement, output);
                }
            }
            Stmt::If(statement) => {
                collect_local_value_assignments(statement.cons.as_ref(), output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_local_value_assignments(alternate, output);
                }
            }
            Stmt::While(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::DoWhile(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::For(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::ForIn(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::ForOf(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_local_value_assignments(statement, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_local_value_assignments(statement, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_local_value_assignments(statement, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_local_value_assignments(statement, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_local_value_assignments(statement.body.as_ref(), output)
            }
            _ => {}
        }
    }

    fn widened_local_kind(
        name: &Ident,
        initializer: &Expr,
        assignments: &[&Expr],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<(JitKind, std::collections::HashSet<JitKind>)> {
        let mut encoded = Vec::new();
        encode_expression(initializer, parameters, locals, context, &mut encoded)?;
        let mut kind = if boolean_literal(initializer) {
            JitKind::Boolean
        } else {
            jit_expression_kind(&encoded)?.0
        };
        let mut candidates: std::collections::HashSet<JitKind> = if kind == JitKind::Dynamic {
            encoded
                .iter()
                .filter_map(|token| match token.as_str() {
                    "tagnum" => Some(JitKind::Number),
                    "tagbool" => Some(JitKind::Boolean),
                    "tagstr" => Some(JitKind::String),
                    _ => None,
                })
                .collect()
        } else {
            std::iter::once(kind).collect()
        };
        if !matches!(
            kind,
            JitKind::Number | JitKind::Boolean | JitKind::String | JitKind::Dynamic
        ) {
            return None;
        }
        let mut probe_locals = locals.clone();
        for assignment in assignments {
            probe_locals.insert(
                name.sym.to_string(),
                vec![match kind {
                    JitKind::Number => "a0",
                    JitKind::Boolean => "b0",
                    JitKind::String => "s0",
                    JitKind::Dynamic => "ld0",
                    _ => return None,
                }
                .into()],
            );
            let mut encoded = Vec::new();
            encode_expression(
                assignment,
                parameters,
                &probe_locals,
                context,
                &mut encoded,
            )?;
            let assignment_kind = if boolean_literal(assignment) {
                JitKind::Boolean
            } else {
                jit_expression_kind(&encoded)?.0
            };
            if assignment_kind == JitKind::Dynamic {
                candidates.extend(encoded.iter().filter_map(|token| match token.as_str() {
                    "tagnum" => Some(JitKind::Number),
                    "tagbool" => Some(JitKind::Boolean),
                    "tagstr" => Some(JitKind::String),
                    _ => None,
                }));
            } else {
                candidates.insert(assignment_kind);
            }
            kind = merge_jit_kinds(kind, assignment_kind)?;
        }
        (kind == JitKind::Dynamic).then_some((kind, candidates))
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_loop_steps(
        steps: Vec<LocalStep<'_>>,
        loop_body: &Stmt,
        parameters: &std::collections::HashMap<String, String>,
        locals: &mut std::collections::HashMap<String, Vec<String>>,
        mutable: &mut std::collections::HashSet<String>,
        kinds: &mut std::collections::HashMap<String, JitKind>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut candidates = std::collections::HashMap::new();
        collect_loop_callable_assignments(loop_body, context, &mut candidates);
        let mut value_assignments = std::collections::HashMap::new();
        collect_local_value_assignments(loop_body, &mut value_assignments);
        for step in steps {
            if let LocalStep::Declare {
                name,
                initializer,
                mutable: is_mutable,
            } = &step
            {
                let mut selection = Vec::new();
                let mut unused_local = 0usize;
                if let Some(alias) = encode_callable_member_snapshot(
                    initializer,
                    parameters,
                    locals,
                    context,
                    &mut unused_local,
                    &mut selection,
                ) {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    let (_, helpers) = callable_local_alias(&[alias])?;
                    let index = kinds.len();
                    output.extend(selection);
                    locals.insert(
                        name.sym.to_string(),
                        vec![format!(
                            "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                            helpers.join("|")
                        )],
                    );
                    kinds.insert(name.sym.to_string(), JitKind::Number);
                    if *is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    continue;
                }
                if let Some(initial_helpers) = finite_callable_names(initializer).filter(|helpers| {
                    helpers
                        .iter()
                        .all(|helper| context.helpers.contains_key(helper))
                }) {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    let mut helpers = initial_helpers;
                    for helper in candidates
                        .remove(name.sym.as_ref())
                        .into_iter()
                        .flatten()
                    {
                        if !helpers.contains(&helper) {
                            helpers.push(helper);
                        }
                    }
                    if helpers.len() == 1 {
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!("{CALLABLE_ALIAS_PREFIX}{}", helpers[0])],
                        );
                    } else {
                        let index = kinds.len();
                        encode_callable_assignment_selection(
                            initializer,
                            &helpers,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!(
                                "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                                helpers.join("|")
                            )],
                        );
                        kinds.insert(name.sym.to_string(), JitKind::Number);
                    }
                    if *is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    continue;
                }
            }
            let forced_kind = match &step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: true,
                } => value_assignments
                    .remove(name.sym.as_ref())
                    .and_then(|assignments| {
                        widened_local_kind(
                            name,
                            initializer,
                            &assignments,
                            parameters,
                            locals,
                            context,
                        )
                    }),
                _ => None,
            };
            encode_loop_declaration(
                step,
                forced_kind,
                parameters,
                locals,
                mutable,
                kinds,
                context,
                output,
            )?;
        }
        Some(())
    }

    fn encode_while_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::While(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        output.push("loop".into());
        output.push("looptail".into());
        encode_condition(
            loop_statement.test.as_ref(), parameters, &locals, context, output,
        )?;
        output.push("while".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_for_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::For(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        match loop_statement.init.as_ref() {
            Some(VarDeclOrExpr::VarDecl(declaration)) => {
                for declarator in &declaration.decls {
                    let Pat::Ident(name) = &declarator.name else {
                        return None;
                    };
                    encode_loop_declaration(
                        LocalStep::Declare {
                            name: &name.id,
                            initializer: declarator.init.as_deref()?,
                            mutable: declaration.kind != VarDeclKind::Const,
                        },
                        None,
                        parameters,
                        &mut locals,
                        &mut mutable,
                        &mut kinds,
                        context,
                        output,
                    )?;
                }
            }
            Some(VarDeclOrExpr::Expr(initializer)) => encode_loop_expression(
                initializer.as_ref(),
                parameters,
                &locals,
                &mutable,
                &kinds,
                context,
                output,
            )?,
            None => {}
        }
        output.push("loop".into());
        if let Some(test) = loop_statement.test.as_deref() {
            encode_condition(test, parameters, &locals, context, output)?;
        } else {
            output.extend([
                format!("c{:016x}", 1.0f64.to_bits()),
                "asbool".into(),
            ]);
        }
        output.push("while".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        if let Some(update) = loop_statement.update.as_deref() {
            encode_loop_expression(
                update,
                parameters,
                &locals,
                &mutable,
                &kinds,
                context,
                output,
            )?;
        }
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_do_while_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::DoWhile(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        output.push("loop".into());
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        encode_condition(
            loop_statement.test.as_ref(), parameters, &locals, context, output,
        )?;
        output.push("while".into());
        output.push("loopend".into());
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_for_of_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::ForOf(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        if loop_statement.is_await {
            return None;
        }
        let mut source = Vec::new();
        encode_expression(
            loop_statement.right.as_ref(), parameters, &locals, context, &mut source,
        )?;
        if jit_expression_kind(&source)?.0 != JitKind::Array {
            return None;
        }
        let array = array_prefix(&source)?;
        let element_kind = match array {
            "rn" => JitKind::Number,
            "rb" => JitKind::Boolean,
            "rs" => JitKind::String,
            _ => return None,
        };
        let source_index = kinds.len();
        output.extend(source);
        kinds.insert(format!("\0forof-source-{source_index}"), JitKind::Array);
        let source_local = format!("{array}l{source_index}");

        let index = kinds.len();
        output.push(format!("c{:016x}", 0.0f64.to_bits()));
        kinds.insert(format!("\0forof-index-{index}"), JitKind::Number);
        let index_local = format!("ln{index}");

        let element_index = match &loop_statement.left {
            ForHead::VarDecl(declaration) => {
                let [declarator] = declaration.decls.as_slice() else {
                    return None;
                };
                let Pat::Ident(name) = &declarator.name else {
                    return None;
                };
                if declarator.init.is_some()
                    || parameters.contains_key(name.id.sym.as_ref())
                    || locals.contains_key(name.id.sym.as_ref())
                {
                    return None;
                }
                match element_kind {
                    JitKind::String => encode_string("", output)?,
                    JitKind::Number | JitKind::Boolean => {
                        output.push(format!("c{:016x}", 0.0f64.to_bits()));
                        if element_kind == JitKind::Boolean {
                            output.push("asbool".into());
                        }
                    }
                    _ => return None,
                }
                let element_index = kinds.len();
                let prefix = match element_kind {
                    JitKind::Number => "ln",
                    JitKind::Boolean => "lb",
                    JitKind::String => "ls",
                    _ => return None,
                };
                let element_local = format!("{prefix}{element_index}");
                locals.insert(name.id.sym.to_string(), vec![element_local.clone()]);
                kinds.insert(name.id.sym.to_string(), element_kind);
                if declaration.kind != VarDeclKind::Const {
                    mutable.insert(name.id.sym.to_string());
                }
                element_index
            }
            ForHead::Pat(pattern) => {
                let Pat::Ident(name) = pattern.as_ref() else {
                    return None;
                };
                if !mutable.contains(name.id.sym.as_ref())
                    || kinds.get(name.id.sym.as_ref())? != &element_kind
                {
                    return None;
                }
                loop_local_index(locals.get(name.id.sym.as_ref())?.first()?)?
            }
            ForHead::UsingDecl(_) => return None,
        };

        output.push("loop".into());
        output.extend([index_local.clone(), source_local.clone(), "arraylen".into(), "<".into()]);
        output.push("while".into());
        output.extend([
            source_local.clone(),
            index_local.clone(),
            format!("{array}get"),
            format!("setl{element_index}"),
        ]);
        encode_loop_effects(
            loop_statement.body.as_ref(),
            parameters,
            &locals,
            &mutable,
            (&kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        output.push("looptail".into());
        output.extend([
            index_local,
            format!("c{:016x}", 1.0f64.to_bits()),
            "+".into(),
            format!("setl{index}"),
            "loopend".into(),
        ]);
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_for_in_body(
        steps: Vec<LocalStep<'_>>,
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (first, rest) = statements.split_first()?;
        let Stmt::ForIn(loop_statement) = first else {
            return None;
        };
        let mut mutable = std::collections::HashSet::new();
        let mut kinds = std::collections::HashMap::new();
        encode_callable_loop_steps(
            steps,
            loop_statement.body.as_ref(),
            parameters,
            &mut locals,
            &mut mutable,
            &mut kinds,
            context,
            output,
        )?;
        encode_for_in_loop(
            loop_statement,
            parameters,
            &mut locals,
            &mut mutable,
            (&mut kinds, root_loop_control(), &[]),
            context,
            output,
        )?;
        let mut tail = Vec::new();
        encode_returning_statements(rest, parameters, &locals, context, &mut tail)?;
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), kinds.len()));
        (output.len() <= 256).then_some(())
    }

    fn encode_steps_and_body(
        steps: Vec<LocalStep<'_>>,
        body: NumericBody<'_>,
        parameters: &std::collections::HashMap<String, String>,
        mut locals: std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let NumericBody::Statements(statements) = &body {
            if matches!(statements.first(), Some(Stmt::While(_))) {
                return encode_while_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::For(_))) {
                return encode_for_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::DoWhile(_))) {
                return encode_do_while_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::ForOf(_))) {
                return encode_for_of_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
            if matches!(statements.first(), Some(Stmt::ForIn(_))) {
                return encode_for_in_body(
                    steps, statements, parameters, locals, context, output,
                );
            }
        }
        let mut mutable = std::collections::HashSet::new();
        let mut runtime_locals = 0usize;
        let mut runtime_kinds = std::collections::HashMap::new();
        let materialize_control_locals = matches!(
            &body,
            NumericBody::Statements(
                [Stmt::Block(_)
                    | Stmt::If(_)
                    | Stmt::Switch(_)
                    | Stmt::Try(_), ..]
                )
        );
        let mut control_callable_candidates = std::collections::HashMap::new();
        let mut control_value_assignments = std::collections::HashMap::new();
        if materialize_control_locals {
            let NumericBody::Statements(statements) = &body else {
                return None;
            };
            for statement in *statements {
                collect_loop_callable_assignments(
                    statement,
                    context,
                    &mut control_callable_candidates,
                );
                collect_local_value_assignments(statement, &mut control_value_assignments);
            }
        }
        for step in steps {
            match step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: is_mutable,
                } => {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    if let Some(mut alias) = encode_callable_member_snapshot(
                        initializer,
                        parameters,
                        &locals,
                        context,
                        &mut runtime_locals,
                        output,
                    ) {
                        if materialize_control_locals {
                            let (local, source_helpers) =
                                callable_local_alias(std::slice::from_ref(&alias))?;
                            let mut helpers = source_helpers.clone();
                            for helper in control_callable_candidates
                                .remove(name.sym.as_ref())
                                .into_iter()
                                .flatten()
                            {
                                if !helpers.contains(&helper) {
                                    helpers.push(helper);
                                }
                            }
                            if helpers != source_helpers {
                                encode_callable_selection_remap(
                                    &source_helpers,
                                    &helpers,
                                    output,
                                )?;
                                alias = format!(
                                    "{CALLABLE_LOCAL_PREFIX}{local}:{}",
                                    helpers.join("|")
                                );
                            }
                        }
                        locals.insert(name.sym.to_string(), vec![alias]);
                        runtime_kinds.insert(name.sym.to_string(), JitKind::Number);
                        if runtime_kinds.len() != runtime_locals {
                            return None;
                        }
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if let Some(alias) = encode_callable_member_alias(
                        initializer,
                        parameters,
                        &locals,
                        context,
                    ) {
                        locals.insert(name.sym.to_string(), vec![alias]);
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if materialize_control_locals {
                        if let Some(mut helpers) = finite_callable_names(initializer).filter(
                            |helpers| {
                                helpers
                                    .iter()
                                    .all(|helper| context.helpers.contains_key(helper))
                            },
                        ) {
                            for helper in control_callable_candidates
                                .remove(name.sym.as_ref())
                                .into_iter()
                                .flatten()
                            {
                                if !helpers.contains(&helper) {
                                    helpers.push(helper);
                                }
                            }
                            if helpers.len() > 1 {
                                encode_callable_assignment_selection(
                                    initializer,
                                    &helpers,
                                    parameters,
                                    &locals,
                                    context,
                                    output,
                                )?;
                                let index = runtime_kinds.len();
                                locals.insert(
                                    name.sym.to_string(),
                                    vec![format!(
                                        "{CALLABLE_LOCAL_PREFIX}ln{index}:{}",
                                        helpers.join("|")
                                    )],
                                );
                                runtime_kinds.insert(name.sym.to_string(), JitKind::Number);
                                runtime_locals = runtime_kinds.len();
                                if is_mutable {
                                    mutable.insert(name.sym.to_string());
                                }
                                continue;
                            }
                        }
                    }
                    if let Some(alias) = callable_alias(initializer, &locals, context) {
                        locals.insert(
                            name.sym.to_string(),
                            vec![format!("{CALLABLE_ALIAS_PREFIX}{alias}")],
                        );
                        if is_mutable {
                            mutable.insert(name.sym.to_string());
                        }
                        continue;
                    }
                    if materialize_control_locals && is_mutable {
                        let forced_kind = control_value_assignments
                            .remove(name.sym.as_ref())
                            .and_then(|assignments| {
                                widened_local_kind(
                                    name,
                                    initializer,
                                    &assignments,
                                    parameters,
                                    &locals,
                                    context,
                                )
                            });
                        encode_loop_declaration(
                            LocalStep::Declare {
                                name,
                                initializer,
                                mutable: true,
                            },
                            forced_kind,
                            parameters,
                            &mut locals,
                            &mut mutable,
                            &mut runtime_kinds,
                            context,
                            output,
                        )?;
                        runtime_locals = runtime_kinds.len();
                        continue;
                    }
                    let mut encoded = Vec::new();
                    encode_expression(initializer, parameters, &locals, context, &mut encoded)?;
                    if !stable_jit_tokens(&encoded) {
                        return None;
                    }
                    locals.insert(name.sym.to_string(), encoded);
                    if is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                }
                LocalStep::Assign {
                    name,
                    operation,
                    value,
                } => {
                    if let Some(slot) = context.module_globals.get(name.sym.as_ref()).copied() {
                        let mut encoded = vec![format!("c{:016x}", (slot as f64).to_bits())];
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(value, parameters, &locals, context, &mut encoded)?;
                        if operation != AssignOp::Assign {
                            encoded.push(
                                match operation {
                                    AssignOp::AddAssign => "+",
                                    AssignOp::SubAssign => "-",
                                    AssignOp::MulAssign => "*",
                                    AssignOp::DivAssign => "/",
                                    AssignOp::ModAssign => "%",
                                    AssignOp::LShiftAssign => "shl",
                                    AssignOp::RShiftAssign => "shr",
                                    AssignOp::ZeroFillRShiftAssign => "ushr",
                                    AssignOp::BitOrAssign => "bor",
                                    AssignOp::BitXorAssign => "bxor",
                                    AssignOp::BitAndAssign => "band",
                                    AssignOp::ExpAssign => "pow",
                                    _ => return None,
                                }
                                .into(),
                            );
                        }
                        encoded.extend(["globalset".into(), "drop".into()]);
                        output.extend(encoded);
                        continue;
                    }
                    if runtime_kinds.contains_key(name.sym.as_ref()) {
                        encode_local_assignment(
                            name,
                            operation,
                            value,
                            parameters,
                            &locals,
                            &mutable,
                            &runtime_kinds,
                            context,
                            output,
                        )?;
                        continue;
                    }
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    if operation == AssignOp::Assign {
                        if let Some((local, helpers)) = locals
                            .get(name.sym.as_ref())
                            .and_then(|tokens| callable_local_alias(tokens))
                        {
                            let mut selection = Vec::new();
                            let mut unused_local = 0usize;
                            if let Some(alias) = encode_callable_member_snapshot(
                                value,
                                parameters,
                                &locals,
                                context,
                                &mut unused_local,
                                &mut selection,
                            ) {
                                let (_, selected_helpers) =
                                    callable_local_alias(&[alias])?;
                                if helpers != selected_helpers {
                                    return None;
                                }
                                output.extend(selection);
                                output.push(format!("setl{}", loop_local_index(&local)?));
                                continue;
                            }
                        }
                        if let Some(alias) = encode_callable_member_alias(
                            value,
                            parameters,
                            &locals,
                            context,
                        ) {
                            locals.insert(name.sym.to_string(), vec![alias]);
                            continue;
                        }
                        if let Some(alias) = callable_alias(value, &locals, context) {
                            locals.insert(
                                name.sym.to_string(),
                                vec![format!("{CALLABLE_ALIAS_PREFIX}{alias}")],
                            );
                            continue;
                        }
                    }
                    let mut encoded = Vec::new();
                    if operation == AssignOp::AddAssign {
                        let mut right = Vec::new();
                        encode_expression(value, parameters, &locals, context, &mut right)?;
                        append_add(
                            locals.get(name.sym.as_ref())?.clone(),
                            right,
                            &mut encoded,
                        )?;
                    } else {
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(value, parameters, &locals, context, &mut encoded)?;
                    }
                    if operation != AssignOp::Assign && operation != AssignOp::AddAssign {
                        encoded.push(
                            match operation {
                                AssignOp::SubAssign => "-",
                                AssignOp::MulAssign => "*",
                                AssignOp::DivAssign => "/",
                                AssignOp::ModAssign => "%",
                                AssignOp::LShiftAssign => "shl",
                                AssignOp::RShiftAssign => "shr",
                                AssignOp::ZeroFillRShiftAssign => "ushr",
                                AssignOp::BitOrAssign => "bor",
                                AssignOp::BitXorAssign => "bxor",
                                AssignOp::BitAndAssign => "band",
                                AssignOp::ExpAssign => "pow",
                                _ => return None,
                            }
                            .into(),
                        );
                    }
                    if !stable_jit_tokens(&encoded) {
                        return None;
                    }
                    locals.insert(name.sym.to_string(), encoded);
                }
                LocalStep::Update { name, operation } => {
                    if let Some(slot) = context.module_globals.get(name.sym.as_ref()).copied() {
                        output.push(format!("c{:016x}", (slot as f64).to_bits()));
                        output.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        output.push(format!("c{:016x}", 1.0f64.to_bits()));
                        output.push(
                            match operation {
                                UpdateOp::PlusPlus => "+",
                                UpdateOp::MinusMinus => "-",
                            }
                            .into(),
                        );
                        output.extend(["globalset".into(), "drop".into()]);
                        continue;
                    }
                    if runtime_kinds.contains_key(name.sym.as_ref()) {
                        encode_local_update(
                            name,
                            operation,
                            &locals,
                            &mutable,
                            &runtime_kinds,
                            output,
                        )?;
                        continue;
                    }
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    let mut encoded = locals.get(name.sym.as_ref())?.clone();
                    encoded.push(format!("c{:016x}", 1.0f64.to_bits()));
                    encoded.push(
                        match operation {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    locals.insert(name.sym.to_string(), encoded);
                }
                LocalStep::Effect(expression) => {
                    let mut effect = Vec::new();
                    if encode_callable_table_update(
                        expression,
                        parameters,
                        &locals,
                        context,
                        &mut effect,
                    )
                    .is_some()
                    {
                        output.extend(effect);
                        continue;
                    }
                    effect.clear();
                    encode_expression(expression, parameters, &locals, context, &mut effect)?;
                    effect.push("drop".into());
                    output.extend(effect);
                }
            }
        }
        let mut tail = Vec::new();
        if let NumericBody::Statements(statements) = body {
            encode_callable_alias_flow(
                statements,
                parameters,
                &locals,
                &mutable,
                context,
                &mut tail,
            )?;
        } else {
            encode_numeric_body(body, parameters, &locals, context, &mut tail)?;
        }
        output.extend(tail);
        output.extend(std::iter::repeat_n("nip".into(), runtime_locals));
        Some(())
    }

    fn encode_callable_alias_flow(
        statements: &[Stmt],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        mutable: &std::collections::HashSet<String>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let [statement @ (Stmt::Block(_)
            | Stmt::If(_)
            | Stmt::Switch(_)
            | Stmt::Try(_)
            | Stmt::While(_)
            | Stmt::DoWhile(_)
            | Stmt::For(_)
            | Stmt::ForOf(_)
            | Stmt::ForIn(_)), rest @ ..] = statements
        {
            let kinds = callable_runtime_kinds(locals)?;
            if !kinds.is_empty() {
                let mut control_flow = Vec::new();
                if encode_loop_effects(
                    statement,
                    parameters,
                    locals,
                    mutable,
                    (&kinds, root_loop_control(), &[]),
                    context,
                    &mut control_flow,
                )
                .is_some()
                {
                    let mut tail = Vec::new();
                    encode_callable_alias_flow(
                        rest, parameters, locals, mutable, context, &mut tail,
                    )?;
                    output.extend(control_flow);
                    output.extend(tail);
                    return Some(());
                }
            }
        }
        let [Stmt::If(branch), rest @ ..] = statements else {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        };
        if rest.is_empty() {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        }
        let Some((name, consequent)) =
            callable_alias_assignment(branch.cons.as_ref(), locals, context)
        else {
            return encode_returning_statements(
                statements,
                parameters,
                locals,
                context,
                output,
            );
        };
        if !mutable.contains(&name) {
            return None;
        }
        let initial = locals
            .get(&name)?
            .as_slice()
            .first()?
            .strip_prefix(CALLABLE_ALIAS_PREFIX)?
            .to_owned();
        let alternate = if let Some(statement) = branch.alt.as_deref() {
            let (alternate_name, alternate) =
                callable_alias_assignment(statement, locals, context)?;
            (alternate_name == name).then_some(alternate)?
        } else {
            initial
        };
        encode_condition(
            branch.test.as_ref(),
            parameters,
            locals,
            context,
            output,
        )?;
        output.push("if".into());
        let mut selected = locals.clone();
        selected.insert(
            name.clone(),
            vec![format!("{CALLABLE_ALIAS_PREFIX}{consequent}")],
        );
        encode_callable_alias_flow(
            rest,
            parameters,
            &selected,
            mutable,
            context,
            output,
        )?;
        output.push("else".into());
        selected.insert(
            name,
            vec![format!("{CALLABLE_ALIAS_PREFIX}{alternate}")],
        );
        encode_callable_alias_flow(
            rest,
            parameters,
            &selected,
            mutable,
            context,
            output,
        )?;
        output.push("end".into());
        Some(())
    }

    fn callable_runtime_kinds(
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<std::collections::HashMap<String, JitKind>> {
        let mut indexed = Vec::new();
        for (name, tokens) in locals {
            let (local, kind) = if let Some((local, _)) = callable_local_alias(tokens) {
                (local, JitKind::Number)
            } else {
                let Some(local) = tokens.first() else {
                    continue;
                };
                let Some(kind) = runtime_local_kind(local) else {
                    continue;
                };
                (local.clone(), kind)
            };
            let index = loop_local_index(&local)?;
            if indexed.len() <= index {
                indexed.resize(index + 1, None);
            }
            if indexed[index].replace((name.clone(), kind)).is_some() {
                return None;
            }
        }
        indexed.into_iter().collect()
    }

    fn runtime_local_kind(local: &str) -> Option<JitKind> {
        let prefix = local.get(..local.find(|character: char| character.is_ascii_digit())?)?;
        match prefix {
            "ln" => Some(JitKind::Number),
            "lb" => Some(JitKind::Boolean),
            "ls" => Some(JitKind::String),
            "ld" => Some(JitKind::Dynamic),
            "rnl" | "rbl" | "rsl" => Some(JitKind::Array),
            "dnl" | "dbl" | "dsl" => Some(JitKind::Dictionary),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_local_reassignment(
        expression: &Expr,
        local: &str,
        helpers: &[String],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut selection = Vec::new();
        let mut unused_local = 0usize;
        if let Some(alias) = encode_callable_member_snapshot(
            expression,
            parameters,
            locals,
            context,
            &mut unused_local,
            &mut selection,
        ) {
            let (_, selected_helpers) = callable_local_alias(&[alias])?;
            if helpers != selected_helpers {
                encode_callable_selection_remap(&selected_helpers, helpers, &mut selection)?;
            }
        } else {
            encode_callable_assignment_selection(
                expression,
                helpers,
                parameters,
                locals,
                context,
                &mut selection,
            )?;
        }
        output.extend(selection);
        output.push(format!("setl{}", loop_local_index(local)?));
        Some(())
    }

    fn encode_callable_selection_remap(
        source_helpers: &[String],
        target_helpers: &[String],
        output: &mut Vec<String>,
    ) -> Option<()> {
        if target_helpers.starts_with(source_helpers) {
            return Some(());
        }
        let branches = source_helpers
            .iter()
            .map(|source| {
                let target = target_helpers.iter().position(|helper| helper == source)?;
                Some(vec![format!("c{:016x}", (target as f64).to_bits())])
            })
            .collect::<Option<Vec<_>>>()?;
        encode_dynamic_callable_branches(&branches, 0, "dup", output)?;
        output.push("nip".into());
        Some(())
    }

    fn callable_alias_assignment(
        statement: &Stmt,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<(String, String)> {
        if let Stmt::Block(block) = statement {
            let [statement] = block.stmts.as_slice() else {
                return None;
            };
            return callable_alias_assignment(statement, locals, context);
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        let Expr::Assign(assignment) = statement.expr.as_ref() else {
            return None;
        };
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left else {
            return None;
        };
        Some((
            name.id.sym.to_string(),
            callable_alias(assignment.right.as_ref(), locals, context)?,
        ))
    }

    fn encode_numeric_body(
        body: NumericBody<'_>,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match body {
            NumericBody::Expression(body) => {
                encode_expression(body, parameters, locals, context, output)?;
            }
            NumericBody::Statements(statements) => {
                encode_returning_statements(statements, parameters, locals, context, output)?;
            }
        }
        Some(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ExportStyle {
        Whole,
        Named,
    }

    #[derive(Clone, Copy)]
    enum NumericCallable<'a> {
        Function(&'a Function),
        Arrow(&'a ArrowExpr),
    }

    struct InlineContext<'a> {
        helpers: &'a std::collections::HashMap<String, NumericCallable<'a>>,
        module_locals: std::collections::HashMap<String, Vec<String>>,
        module_globals: std::collections::HashMap<String, u8>,
        callable_tables: std::collections::HashMap<String, Vec<(String, String)>>,
        callable_table_states:
            std::collections::HashMap<String, std::collections::HashMap<String, (u8, Vec<String>)>>,
        dynamic_callable_tables: std::collections::HashMap<String, (u8, Vec<String>)>,
        active: Vec<String>,
        recursive_names: Vec<String>,
        recursive_parameters: Vec<thaw_hir::HirType>,
        recursive_result: Option<JitKind>,
    }

    fn same_callable(left: NumericCallable<'_>, right: NumericCallable<'_>) -> bool {
        match (left, right) {
            (NumericCallable::Function(left), NumericCallable::Function(right)) => {
                std::ptr::eq(left, right)
            }
            (NumericCallable::Arrow(left), NumericCallable::Arrow(right)) => {
                std::ptr::eq(left, right)
            }
            _ => false,
        }
    }

    fn callable_parts(callable: NumericCallable<'_>) -> Option<(Vec<&Pat>, Vec<LocalStep<'_>>, NumericBody<'_>)> {
        match callable {
            NumericCallable::Function(function)
                if !function.is_async && !function.is_generator =>
            {
                let body = function.body.as_ref()?;
                let (locals, body) = split_numeric_body(&body.stmts)?;
                Some((
                    function.params.iter().map(|parameter| &parameter.pat).collect(),
                    locals,
                    body,
                ))
            }
            NumericCallable::Arrow(function)
                if !function.is_async && !function.is_generator =>
            {
                let (locals, body) = match function.body.as_ref() {
                    thaw_parser::ast::ArrowFunctionBody::Expr(body) => {
                        (Vec::new(), NumericBody::Expression(body.as_ref()))
                    }
                    thaw_parser::ast::ArrowFunctionBody::FunctionBody(body) => {
                        split_numeric_body(&body.stmts)?
                    }
                };
                Some((function.params.iter().collect(), locals, body))
            }
            _ => None,
        }
    }

    fn numeric_reducer(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(accumulator), Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if accumulator.id.sym == value.id.sym {
            return None;
        }
        if !steps.is_empty() {
            let [LocalStep::Declare {
                name,
                mutable: false,
                ..
            }] = steps.as_slice()
            else {
                return None;
            };
            let NumericBody::Statements([Stmt::Return(statement)]) = &body else {
                return None;
            };
            if !matches!(statement.arg.as_deref(), Some(Expr::Ident(result)) if result.sym == name.sym)
            {
                return None;
            }
        }
        let callback_parameters = std::collections::HashMap::from([
            (accumulator.id.sym.to_string(), "a0".into()),
            (value.id.sym.to_string(), "a1".into()),
        ]);
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            context.module_locals.clone(),
            context,
            &mut encoded,
        )?;
        let [left, right, operation] = encoded.as_slice() else {
            return None;
        };
        if left != "a0" || right != "a1" {
            return None;
        }
        match operation.as_str() {
            "+" => Some("add"),
            "-" => Some("sub"),
            "*" => Some("mul"),
            "/" => Some("div"),
            "%" => Some("rem"),
            "pow" => Some("pow"),
            "min" | "max"
                if !outer_parameters.contains_key("Math")
                    && !outer_locals.contains_key("Math")
                    && !context.helpers.contains_key("Math") =>
            {
                match operation.as_str() {
                    "min" => Some("min"),
                    "max" => Some("max"),
                    _ => unreachable!(),
                }
            }
            _ => None,
        }
    }

    fn numeric_sort_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<bool> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(left), Pat::Ident(right)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() || left.id.sym == right.id.sym {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        if binary.op != BinaryOp::Sub {
            return None;
        }
        if matches!(binary.left.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
        {
            Some(false)
        } else if matches!(binary.left.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
        {
            Some(true)
        } else {
            None
        }
    }

    fn string_sort_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<bool> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(left), Pat::Ident(right)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() || left.id.sym == right.id.sym {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Call(call) = expression else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        let [argument] = call.args.as_slice() else {
            return None;
        };
        if argument.spread.is_some() || property.sym != "localeCompare" {
            return None;
        }
        if matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
        {
            Some(false)
        } else if matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == right.id.sym)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == left.id.sym)
        {
            Some(true)
        } else {
            None
        }
    }

    fn encode_numeric_quantifier_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let (operand, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (binary.right.as_ref(), false)
        } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym) {
            (binary.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match (binary.op, reverse) {
            (BinaryOp::Lt, false) | (BinaryOp::Gt, true) => "lt",
            (BinaryOp::LtEq, false) | (BinaryOp::GtEq, true) => "lte",
            (BinaryOp::Gt, false) | (BinaryOp::Lt, true) => "gt",
            (BinaryOp::GtEq, false) | (BinaryOp::LtEq, true) => "gte",
            (BinaryOp::EqEq | BinaryOp::EqEqEq, _) => "eq",
            (BinaryOp::NotEq | BinaryOp::NotEqEq, _) => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(
            operand,
            outer_parameters,
            outer_locals,
            context,
            &mut encoded,
        )?;
        if encoded
            .iter()
            .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        if matches!(binary.op, BinaryOp::EqEqEq | BinaryOp::NotEqEq)
            && jit_expression_kind(&encoded)?.0 != JitKind::Number
        {
            return None;
        }
        append_number(encoded, output)?;
        Some(operation)
    }

    fn encode_primitive_comparison_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        expected: JitKind,
        output: &mut Vec<String>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let (operand, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (binary.right.as_ref(), false)
        } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym) {
            (binary.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match (binary.op, reverse) {
            (BinaryOp::Lt, false) | (BinaryOp::Gt, true) => "lt",
            (BinaryOp::LtEq, false) | (BinaryOp::GtEq, true) => "lte",
            (BinaryOp::Gt, false) | (BinaryOp::Lt, true) => "gt",
            (BinaryOp::GtEq, false) | (BinaryOp::LtEq, true) => "gte",
            (BinaryOp::EqEq, _) => "eq",
            (BinaryOp::NotEq, _) => "ne",
            (BinaryOp::EqEqEq, _) if expected == JitKind::Dynamic => "seq",
            (BinaryOp::NotEqEq, _) if expected == JitKind::Dynamic => "sne",
            (BinaryOp::EqEqEq, _) => "eq",
            (BinaryOp::NotEqEq, _) => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(operand, outer_parameters, outer_locals, context, &mut encoded)?;
        if jit_expression_kind(&encoded)?.0 != expected {
            return None;
        }
        output.extend(encoded);
        Some(operation)
    }

    fn primitive_truthy_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> bool {
        let boolean_is_shadowed = outer_parameters.contains_key("Boolean")
            || outer_locals.contains_key("Boolean")
            || context.module_locals.contains_key("Boolean")
            || context.helpers.contains_key("Boolean");
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == "Boolean") {
            return !boolean_is_shadowed;
        }
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return false;
        }
        let Some(callable) = resolve_callable(expression, context.helpers) else {
            return false;
        };
        let Some((parameters, steps, body)) = callable_parts(callable) else {
            return false;
        };
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return false;
        };
        if !steps.is_empty() {
            return false;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => {
                let Some(expression) = statement.arg.as_deref() else {
                    return false;
                };
                expression
            }
            _ => return false,
        };
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
            return true;
        }
        if matches!(expression, Expr::Unary(outer)
            if outer.op == UnaryOp::Bang
                && matches!(outer.arg.as_ref(), Expr::Unary(inner)
                    if inner.op == UnaryOp::Bang
                        && matches!(inner.arg.as_ref(), Expr::Ident(identifier)
                            if identifier.sym == value.id.sym)))
        {
            return true;
        }
        let Expr::Call(call) = expression else {
            return false;
        };
        let Callee::Expr(callee) = &call.callee else {
            return false;
        };
        let [argument] = call.args.as_slice() else {
            return false;
        };
        !boolean_is_shadowed
            && value.id.sym != "Boolean"
            && matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Boolean")
            && argument.spread.is_none()
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym)
    }

    fn encode_numeric_map_operand(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<(&'static str, bool)> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        let marker = (0..16)
            .map(|index| format!("a{index}"))
            .find(|candidate| {
                !outer_parameters.values().any(|token| token == candidate)
                    && !outer_locals
                        .values()
                        .flatten()
                        .any(|token| token == candidate)
            })?;
        let mut callback_parameters = outer_parameters.clone();
        callback_parameters.insert(value.id.sym.to_string(), marker.clone());
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            outer_locals.clone(),
            context,
            &mut encoded,
        )?;
        if encoded.iter().filter(|token| **token == marker).count() != 1 {
            return None;
        }
        let operation = match encoded.pop()?.as_str() {
            "+" => "add",
            "-" => "sub",
            "*" => "mul",
            "/" => "div",
            "%" => "rem",
            "pow" => "pow",
            _ => return None,
        };
        let reverse = if encoded.first() == Some(&marker) {
            encoded.remove(0);
            false
        } else if encoded.last() == Some(&marker) {
            encoded.pop();
            true
        } else {
            return None;
        };
        if encoded.is_empty()
            || jit_expression_kind(&encoded)?.0 != JitKind::Number
            || encoded
                .iter()
                .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        output.extend(encoded);
        Some((operation, reverse))
    }

    fn encode_numeric_conditional_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<String> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Cond(conditional) = expression else {
            return None;
        };
        let Expr::Bin(comparison) = conditional.test.as_ref() else {
            return None;
        };
        let (operand, reverse) = if matches!(comparison.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
        {
            (comparison.right.as_ref(), false)
        } else if matches!(comparison.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
        {
            (comparison.left.as_ref(), true)
        } else {
            return None;
        };
        let operation = match comparison.op {
            BinaryOp::Lt => "lt",
            BinaryOp::LtEq => "lte",
            BinaryOp::Gt => "gt",
            BinaryOp::GtEq => "gte",
            BinaryOp::EqEq | BinaryOp::EqEqEq => "eq",
            BinaryOp::NotEq | BinaryOp::NotEqEq => "ne",
            _ => return None,
        };
        let mut encoded = Vec::new();
        encode_expression(
            operand,
            outer_parameters,
            outer_locals,
            context,
            &mut encoded,
        )?;
        if jit_expression_kind(&encoded)?.0 != JitKind::Number
            || encoded
                .iter()
                .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
        {
            return None;
        }
        let branch_mode = |branch: &Expr, context: &mut InlineContext<'_>| {
            if matches!(branch, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
                return Some(0u8);
            }
            let mut branch_encoded = Vec::new();
            if encode_expression(
                branch,
                outer_parameters,
                outer_locals,
                context,
                &mut branch_encoded,
            )
            .is_some()
                && branch_encoded == encoded
            {
                return Some(1);
            }
            let Expr::Bin(binary) = branch else {
                return None;
            };
            let (other, reverse) = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
            {
                (binary.right.as_ref(), false)
            } else if matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
            {
                (binary.left.as_ref(), true)
            } else {
                return None;
            };
            let mut other_encoded = Vec::new();
            encode_expression(
                other,
                outer_parameters,
                outer_locals,
                context,
                &mut other_encoded,
            )?;
            if other_encoded != encoded {
                return None;
            }
            let operation = match binary.op {
                BinaryOp::Add => 0,
                BinaryOp::Sub => 1,
                BinaryOp::Mul => 2,
                BinaryOp::Div => 3,
                BinaryOp::Mod => 4,
                BinaryOp::Exp => 5,
                _ => return None,
            };
            Some(if reverse { 8 } else { 2 } + operation)
        };
        let true_branch = branch_mode(conditional.cons.as_ref(), context)?;
        let false_branch = branch_mode(conditional.alt.as_ref(), context)?;
        if true_branch == false_branch {
            return None;
        }
        output.extend(encoded);
        if matches!((true_branch, false_branch), (0, 1) | (1, 0)) {
            return Some(format!(
                "rnmapselect{operation}{}",
                u8::from(reverse) | if true_branch == 0 { 0 } else { 2 }
            ));
        }
        let comparison = match operation {
            "lt" => 0,
            "lte" => 1,
            "gt" => 2,
            "gte" => 3,
            "eq" => 4,
            "ne" => 5,
            _ => unreachable!(),
        };
        let encoded = comparison
            | (u16::from(reverse) << 3)
            | (u16::from(true_branch) << 4)
            | (u16::from(false_branch) << 8);
        Some(format!("rnmapbranch{encoded:03x}"))
    }

    fn numeric_index_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<(&'static str, bool)> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value), Pat::Ident(index)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let reverse = if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == value.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == index.id.sym)
        {
            false
        } else if matches!(binary.left.as_ref(), Expr::Ident(left) if left.sym == index.id.sym)
            && matches!(binary.right.as_ref(), Expr::Ident(right) if right.sym == value.id.sym)
        {
            true
        } else {
            return None;
        };
        Some((
            match binary.op {
                BinaryOp::Add => "add",
                BinaryOp::Sub => "sub",
                BinaryOp::Mul => "mul",
                BinaryOp::Div => "div",
                BinaryOp::Mod => "rem",
                BinaryOp::Exp => "pow",
                _ => return None,
            },
            reverse,
        ))
    }

    fn encode_numeric_jit_callback(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        max_parameters: usize,
        array_parameter: Option<usize>,
        callback_abi: (&str, &str, bool),
    ) -> Option<(Vec<String>, JitKind, Vec<Vec<String>>)> {
        let (element_prefix, array_parameter_prefix, dynamic_array) = callback_abi;
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        if parameters.len() > max_parameters {
            return None;
        }
        let mut callback_parameters = std::collections::HashMap::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let Pat::Ident(parameter) = parameter else {
                return None;
            };
            if callback_parameters
                .insert(
                    parameter.id.sym.to_string(),
                    if dynamic_array {
                        match (max_parameters, array_parameter, index) {
                            (3, _, 0) => "u0nbs".into(),
                            (3, _, 1) => "a2".into(),
                            (3, _, 2) => "u3NBS".into(),
                            (4, Some(3), 0) => "a0".into(),
                            (4, Some(3), 1) => "u1nbs".into(),
                            (4, Some(3), 2) => "a3".into(),
                            (4, Some(3), 3) => "u4NBS".into(),
                            (4, None, 0) => "u0nbs".into(),
                            (4, None, 1) => "u2nbs".into(),
                            (4, None, 2) => "a4".into(),
                            (4, None, 3) => "u5NBS".into(),
                            _ => return None,
                        }
                    } else {
                        format!(
                            "{}{}",
                            if array_parameter == Some(index) {
                                array_parameter_prefix
                            } else if index == 0 {
                                element_prefix
                            } else {
                                "a"
                            },
                            index
                        )
                    },
                )
                .is_some()
            {
                return None;
            }
        }
        let mut captures = outer_parameters
            .iter()
            .filter(|(name, _)| !callback_parameters.contains_key(name.as_str()))
            .map(|(name, token)| (name.clone(), vec![token.clone()]))
            .chain(
                outer_locals
                    .iter()
                    .filter(|(name, _)| !callback_parameters.contains_key(name.as_str()))
                    .map(|(name, tokens)| (name.clone(), tokens.clone())),
            )
            .collect::<Vec<_>>();
        captures.sort_by(|left, right| left.0.cmp(&right.0));
        if dynamic_array {
            captures.retain(|(_, tokens)| {
                jit_expression_kind(tokens).is_some_and(|(kind, _)| {
                    kind != JitKind::Dynamic
                        && (kind != JitKind::Array || array_prefix(tokens).is_some())
                })
            });
        }
        let capture_offset = if dynamic_array {
            max_parameters + if array_parameter.is_some() { 2 } else { 3 }
        } else {
            max_parameters
        };
        if capture_offset + captures.len() > 16 {
            return None;
        }
        let mut callback_locals = context.module_locals.clone();
        let mut capture_tokens = Vec::new();
        for (offset, (name, tokens)) in captures.iter().enumerate() {
            let prefix = match jit_expression_kind(tokens)?.0 {
                JitKind::Number => "a",
                JitKind::Boolean => "b",
                JitKind::String => "s",
                JitKind::Dynamic => return None,
                JitKind::Array => array_prefix(tokens)?,
                JitKind::Dictionary => dictionary_prefix(tokens)?,
            };
            let token = format!("{prefix}{}", capture_offset + offset);
            capture_tokens.push(token.clone());
            if outer_parameters.contains_key(name) {
                callback_parameters.insert(name.clone(), token);
            } else {
                callback_locals.insert(name.clone(), vec![token]);
            }
        }
        let mut encoded = Vec::new();
        encode_steps_and_body(
            steps,
            body,
            &callback_parameters,
            callback_locals,
            context,
            &mut encoded,
        )?;
        let mut selected_captures = Vec::new();
        for ((_, capture), token) in captures.into_iter().zip(capture_tokens) {
            if encoded.iter().any(|encoded| encoded == &token) {
                let prefix = token.trim_end_matches(|character: char| character.is_ascii_digit());
                let replacement = format!("{prefix}{}", capture_offset + selected_captures.len());
                for encoded in &mut encoded {
                    if encoded == &token {
                        *encoded = replacement.clone();
                    }
                }
                selected_captures.push(capture);
            }
        }
        let kind = jit_expression_kind(&encoded)?.0;
        stable_jit_tokens(&encoded).then_some((encoded, kind, selected_captures))
    }

    fn append_jit_captures(captures: Vec<Vec<String>>, output: &mut Vec<String>) {
        output.push("arrayempty".into());
        for capture in captures {
            output.extend(capture);
            output.push("captureappend".into());
        }
    }

    fn stable_jit_tokens(tokens: &[String]) -> bool {
        !tokens
            .iter()
            .any(|token| matches!(token.as_str(), "random" | "datenow" | "performancenow"))
    }

    fn numeric_unary_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        if matches!(expression, Expr::Unary(unary)
            if unary.op == UnaryOp::Minus
                && matches!(unary.arg.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("neg");
        }
        if value.id.sym == "Math"
            || context.helpers.contains_key("Math")
            || context.module_locals.contains_key("Math")
        {
            return None;
        }
        let Expr::Call(call) = expression else {
            return None;
        };
        let [argument] = call.args.as_slice() else {
            return None;
        };
        let operation = math_method(call, outer_parameters, outer_locals)?;
        (argument.spread.is_none()
            && is_unary_math_method(operation)
            && matches!(argument.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        .then_some(operation)
    }

    fn primitive_unary_map(
        expression: &Expr,
        outer_parameters: &std::collections::HashMap<String, String>,
        outer_locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
        boolean: bool,
    ) -> Option<&'static str> {
        if matches!(expression, Expr::Ident(identifier)
            if outer_parameters.contains_key(identifier.sym.as_ref())
                || outer_locals.contains_key(identifier.sym.as_ref()))
        {
            return None;
        }
        let callable = resolve_callable(expression, context.helpers)?;
        let (parameters, steps, body) = callable_parts(callable)?;
        let [Pat::Ident(value)] = parameters.as_slice() else {
            return None;
        };
        if !steps.is_empty() {
            return None;
        }
        let expression = match body {
            NumericBody::Expression(expression) => expression,
            NumericBody::Statements([Stmt::Return(statement)]) => statement.arg.as_deref()?,
            _ => return None,
        };
        if matches!(expression, Expr::Ident(identifier) if identifier.sym == value.id.sym) {
            return Some("identity");
        }
        if boolean
            && matches!(expression, Expr::Unary(unary)
                if unary.op == UnaryOp::Bang
                    && matches!(unary.arg.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("not");
        }
        if !boolean
            && matches!(expression, Expr::Member(member)
                if matches!(&member.prop, MemberProp::Ident(property) if property.sym == "length")
                    && matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym))
        {
            return Some("length");
        }
        let Expr::Call(call) = expression else {
            return None;
        };
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        if boolean
            || !call.args.is_empty()
            || !matches!(member.obj.as_ref(), Expr::Ident(identifier) if identifier.sym == value.id.sym)
        {
            return None;
        }
        match property.sym.as_ref() {
            "toLowerCase" => Some("tolowercase"),
            "toUpperCase" => Some("touppercase"),
            "trim" => Some("trim"),
            "trimStart" => Some("trimstart"),
            "trimEnd" => Some("trimend"),
            _ => None,
        }
    }

    fn primitive_conversion_map(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<&'static str> {
        let Expr::Ident(identifier) = expression else {
            return None;
        };
        let name = identifier.sym.as_ref();
        if parameters.contains_key(name)
            || locals.contains_key(name)
            || context.module_locals.contains_key(name)
            || context.helpers.contains_key(name)
        {
            return None;
        }
        match name {
            "Number" => Some("number"),
            "Boolean" => Some("boolean"),
            "String" => Some("string"),
            _ => None,
        }
    }

    const CALLABLE_ALIAS_PREFIX: &str = "\0call:";
    const CALLABLE_LOCAL_PREFIX: &str = "\0calllocal:";
    const CALLABLE_MEMBER_PREFIX: &str = "\0callmember:";

    fn callable_local_alias(tokens: &[String]) -> Option<(String, Vec<String>)> {
        let [marker] = tokens else {
            return None;
        };
        let (local, helpers) = marker
            .strip_prefix(CALLABLE_LOCAL_PREFIX)?
            .split_once(':')?;
        Some((
            local.to_owned(),
            helpers.split('|').map(str::to_owned).collect(),
        ))
    }

    fn callable_member_alias(tokens: &[String]) -> Option<(String, Vec<String>)> {
        let [marker] = tokens else {
            return None;
        };
        let (table, key) = marker
            .strip_prefix(CALLABLE_MEMBER_PREFIX)?
            .split_once(':')?;
        Some((
            table.to_owned(),
            key.split(';').map(str::to_owned).collect(),
        ))
    }

    fn encode_callable_member_snapshot(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        runtime_locals: &mut usize,
        output: &mut Vec<String>,
    ) -> Option<String> {
        let expression = match expression {
            Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
            expression => expression,
        };
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        let static_key = match &member.prop {
            MemberProp::Ident(property) => Some(property.sym.to_string()),
            MemberProp::Computed(property) => {
                if let Expr::Lit(Lit::Str(value)) = property.expr.as_ref() {
                    Some(value.value.to_string_lossy().into_owned())
                } else {
                    None
                }
            }
            MemberProp::PrivateName(_) => return None,
        };
        if let Some((table_id, helpers)) = context
            .dynamic_callable_tables
            .get(table.sym.as_ref())
            .cloned()
        {
            let local = *runtime_locals;
            *runtime_locals += 1;
            if let Some(key) = static_key.as_deref() {
                encode_string(key, output)?;
            } else {
                let MemberProp::Computed(property) = &member.prop else {
                    unreachable!()
                };
                let mut key = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                append_string(key, output)?;
            }
            let initial = context
                .callable_tables
                .get(table.sym.as_ref())?
                .iter()
                .map(|(key, helper)| {
                    let selection = helpers
                        .iter()
                        .position(|candidate| candidate == helper)?;
                    Some((
                        key.clone(),
                        vec![format!("c{:016x}", (selection as f64).to_bits())],
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            output.extend([
                "dup".into(),
                format!("c{:016x}", (table_id as f64).to_bits()),
                "callableget".into(),
                "dup".into(),
                format!("c{:016x}", (-1.0f64).to_bits()),
                "!=".into(),
                "if".into(),
                "dup".into(),
                "else".into(),
                "dup2".into(),
                "drop".into(),
            ]);
            encode_callable_table_branches(
                &initial,
                &format!("c{:016x}", (-1.0f64).to_bits()),
                output,
            )?;
            output.extend([
                "nip".into(),
                "end".into(),
                "nip".into(),
                "nip".into(),
            ]);
            return Some(format!(
                "{CALLABLE_LOCAL_PREFIX}ln{local}:{}",
                helpers.join("|")
            ));
        }
        let key = static_key?;
        let (slot, helpers) = context
            .callable_table_states
            .get(table.sym.as_ref())?
            .get(&key)?;
        let local = *runtime_locals;
        *runtime_locals += 1;
        output.extend([
            format!("c{:016x}", (*slot as f64).to_bits()),
            "globalget".into(),
        ]);
        Some(format!(
            "{CALLABLE_LOCAL_PREFIX}ln{local}:{}",
            helpers.join("|")
        ))
    }

    fn encode_callable_member_alias(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<String> {
        if let Expr::Paren(parenthesized) = expression {
            return encode_callable_member_alias(
                parenthesized.expr.as_ref(),
                parameters,
                locals,
                context,
            );
        }
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        context.callable_tables.get(table.sym.as_ref())?;
        // Re-dispatching through a mutable table at call time would not preserve
        // JavaScript's get-time function identity. Those aliases need a runtime
        // selector snapshot and deliberately remain on the fallback path here.
        if context
            .dynamic_callable_tables
            .contains_key(table.sym.as_ref())
            || context.callable_table_states.contains_key(table.sym.as_ref())
        {
            return None;
        }
        let key = match &member.prop {
            MemberProp::Ident(property) => {
                let mut encoded = Vec::new();
                encode_string(property.sym.as_ref(), &mut encoded)?;
                encoded
            }
            MemberProp::Computed(property) => {
                let mut encoded = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                encoded
            }
            MemberProp::PrivateName(_) => return None,
        };
        if !matches!(
            jit_expression_kind(&key)?.0,
            JitKind::Number | JitKind::Boolean | JitKind::String
        ) || !stable_jit_tokens(&key)
        {
            return None;
        }
        Some(format!(
            "{CALLABLE_MEMBER_PREFIX}{}:{}",
            table.sym,
            key.join(";")
        ))
    }

    fn callable_alias(
        expression: &Expr,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &InlineContext<'_>,
    ) -> Option<String> {
        match expression {
            Expr::Ident(identifier) => {
                if let Some(local) = locals.get(identifier.sym.as_ref()) {
                    let [marker] = local.as_slice() else {
                        return None;
                    };
                    marker.strip_prefix(CALLABLE_ALIAS_PREFIX).map(str::to_owned)
                } else {
                    context
                        .helpers
                        .contains_key(identifier.sym.as_ref())
                        .then(|| identifier.sym.to_string())
                }
            }
            Expr::Paren(parenthesized) => callable_alias(parenthesized.expr.as_ref(), locals, context),
            _ => None,
        }
    }

    fn encode_helper_target(
        callee: &Expr,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if let Expr::Paren(parenthesized) = callee {
            return encode_helper_target(
                parenthesized.expr.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            );
        }
        if let Expr::Cond(conditional) = callee {
            encode_condition(
                conditional.test.as_ref(),
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("if".into());
            encode_helper_target(
                conditional.cons.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("else".into());
            encode_helper_target(
                conditional.alt.as_ref(),
                arguments,
                parameters,
                locals,
                context,
                output,
            )?;
            output.push("end".into());
            return Some(());
        }
        if let Expr::Member(member) = callee {
            return encode_callable_table_call(
                member, arguments, parameters, locals, context, output,
            );
        }
        if let Expr::Ident(identifier) = callee {
            if let Some((table, key)) = locals
                .get(identifier.sym.as_ref())
                .and_then(|tokens| callable_member_alias(tokens))
            {
                return encode_callable_table_key_call(
                    &table,
                    key,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                );
            }
            if let Some((local, helpers)) = locals
                .get(identifier.sym.as_ref())
                .and_then(|tokens| callable_local_alias(tokens))
            {
                let mut branches = Vec::with_capacity(helpers.len());
                for helper in helpers {
                    let mut encoded = Vec::new();
                    encode_named_helper_target(
                        &helper,
                        arguments,
                        parameters,
                        locals,
                        context,
                        &mut encoded,
                    )?;
                    branches.push(encoded);
                }
                let kind = normalize_callable_branches(&mut branches)?;
                return encode_local_callable_branches(
                    &local,
                    &branches,
                    0,
                    missing_callable(kind),
                    output,
                );
            }
        }
        let name = callable_alias(callee, locals, context)?;
        encode_named_helper_target(
            name.as_str(),
            arguments,
            parameters,
            locals,
            context,
            output,
        )
    }

    fn encode_local_callable_branches(
        local: &str,
        branches: &[Vec<String>],
        selection: usize,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((branch, remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.extend([
            local.into(),
            format!("c{:016x}", (selection as f64).to_bits()),
            "==".into(),
            "if".into(),
        ]);
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_local_callable_branches(local, remaining, selection + 1, missing, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_named_helper_target(
        name: &str,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        if context.recursive_names.iter().any(|recursive| recursive == name) {
            if arguments.len() != context.recursive_parameters.len()
                || arguments.iter().any(|argument| argument.spread.is_some())
            {
                return None;
            }
            let expected_parameters = context.recursive_parameters.clone();
            let mut arity = 0;
            for (argument, expected) in arguments.iter().zip(&expected_parameters) {
                arity += encode_recursive_argument(
                    argument.expr.as_ref(),
                    expected,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
            }
            let result = match context.recursive_result? {
                JitKind::Number => 'n',
                JitKind::Boolean => 'b',
                JitKind::String => 's',
                JitKind::Dynamic => return None,
                JitKind::Array => return None,
                JitKind::Dictionary => return None,
            };
            output.push(format!("recur{result}{arity}"));
            return Some(());
        }
        if parameters.contains_key(name)
            || locals.contains_key(name)
            || context.active.len() >= 16
            || context.active.iter().any(|active| active == name)
        {
            return None;
        }
        let callable = *context.helpers.get(name)?;
        let (helper_parameters, steps, body) = callable_parts(callable)?;
        if helper_parameters.len() != arguments.len()
            || arguments.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        let mut helper_locals = context.module_locals.clone();
        let mut bound_parameters = std::collections::HashSet::new();
        for (parameter, argument) in helper_parameters.iter().zip(arguments) {
            let Pat::Ident(parameter) = parameter else {
                return None;
            };
            if !bound_parameters.insert(parameter.id.sym.to_string()) {
                return None;
            }
            let mut encoded = Vec::new();
            encode_expression(
                argument.expr.as_ref(),
                parameters,
                locals,
                context,
                &mut encoded,
            )?;
            helper_locals.insert(parameter.id.sym.to_string(), encoded);
        }
        context.active.push(name.to_string());
        let result = encode_steps_and_body(
            steps,
            body,
            &std::collections::HashMap::new(),
            helper_locals,
            context,
            output,
        );
        context.active.pop();
        result
    }

    fn encode_callable_table_call(
        member: &thaw_parser::ast::MemberExpr,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        let entries = context.callable_tables.get(table.sym.as_ref())?.clone();
        match &member.prop {
            MemberProp::Ident(property) => {
                let (_, helper) = entries
                    .iter()
                    .find(|(key, _)| key == property.sym.as_ref())?;
                if context
                    .dynamic_callable_tables
                    .contains_key(table.sym.as_ref())
                {
                    let mut branch = Vec::new();
                    encode_named_helper_target(
                        helper,
                        arguments,
                        parameters,
                        locals,
                        context,
                        &mut branch,
                    )?;
                    let kind = jit_expression_kind(&branch)?.0;
                    let missing = missing_callable(kind);
                    encode_string(property.sym.as_ref(), output)?;
                    return encode_dynamic_callable_table_dispatch(
                        table.sym.as_ref(),
                        &[(property.sym.to_string(), branch)],
                        arguments,
                        parameters,
                        locals,
                        context,
                        kind,
                        missing,
                        output,
                    );
                }
                encode_callable_table_entry(
                    table.sym.as_ref(),
                    property.sym.as_ref(),
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                )
            }
            MemberProp::Computed(property) => {
                let mut key = Vec::new();
                encode_expression(
                    property.expr.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut key,
                )?;
                encode_callable_table_key_call(
                    table.sym.as_ref(),
                    key,
                    arguments,
                    parameters,
                    locals,
                    context,
                    output,
                )
            }
            MemberProp::PrivateName(_) => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_key_call(
        table: &str,
        key: Vec<String>,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let entries = context.callable_tables.get(table)?.clone();
        let mut branches = entries
            .iter()
            .map(|(entry, helper)| {
                let mut encoded = Vec::new();
                encode_callable_table_entry(
                    table,
                    entry,
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                Some((entry.clone(), encoded))
            })
            .collect::<Option<Vec<_>>>()?;
        let kind = normalize_callable_branches(branches.iter_mut().map(|(_, branch)| branch))?;
        let missing = missing_callable(kind);
        append_string(key, output)?;
        encode_dynamic_callable_table_dispatch(
            table,
            &branches,
            arguments,
            parameters,
            locals,
            context,
            kind,
            missing,
            output,
        )
    }

    fn missing_callable(kind: JitKind) -> &'static str {
        match kind {
            JitKind::Number => "missingcalln",
            JitKind::Boolean => "missingcallb",
            JitKind::String => "missingcalls",
            JitKind::Dynamic => "missingcalldyn",
            JitKind::Array => "missingcalla",
            JitKind::Dictionary => "missingcalld",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_dynamic_callable_table_dispatch(
        table: &str,
        branches: &[(String, Vec<String>)],
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        kind: JitKind,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((table_id, helpers)) = context.dynamic_callable_tables.get(table).cloned() else {
            encode_callable_table_branches(branches, missing, output)?;
            output.push("nip".into());
            return Some(());
        };
        let mut dynamic = helpers
            .iter()
            .map(|helper| {
                let mut encoded = Vec::new();
                encode_named_helper_target(
                    helper,
                    arguments,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                merge_jit_kinds(jit_expression_kind(&encoded)?.0, kind).map(|_| encoded)
            })
            .collect::<Option<Vec<_>>>()?;
        normalize_callable_branches(&mut dynamic)?;
        output.extend([
            "dup".into(),
            format!("c{:016x}", (table_id as f64).to_bits()),
            "callableget".into(),
            format!("c{:016x}", (-1.0f64).to_bits()),
            "!=".into(),
            "if".into(),
            "dup".into(),
            format!("c{:016x}", (table_id as f64).to_bits()),
            "callableget".into(),
        ]);
        encode_dynamic_callable_branches(&dynamic, 0, missing, output)?;
        output.push("nip".into());
        output.push("else".into());
        encode_callable_table_branches(branches, missing, output)?;
        output.push("end".into());
        output.push("nip".into());
        Some(())
    }

    fn encode_dynamic_callable_branches(
        branches: &[Vec<String>],
        selection: usize,
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((branch, remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.extend([
            "dup".into(),
            format!("c{:016x}", (selection as f64).to_bits()),
            "==".into(),
            "if".into(),
        ]);
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_dynamic_callable_branches(remaining, selection + 1, missing, output)?;
        output.push("end".into());
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_entry(
        table: &str,
        key: &str,
        initial: &str,
        arguments: &[thaw_parser::ast::ExprOrSpread],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some((slot, helpers)) = context
            .callable_table_states
            .get(table)
            .and_then(|entries| entries.get(key))
            .cloned()
        else {
            return encode_named_helper_target(
                initial, arguments, parameters, locals, context, output,
            );
        };
        let mut branches = Vec::with_capacity(helpers.len());
        for helper in helpers {
            let mut encoded = Vec::new();
            encode_named_helper_target(
                &helper,
                arguments,
                parameters,
                locals,
                context,
                &mut encoded,
            )?;
            branches.push(encoded);
        }
        normalize_callable_branches(&mut branches)?;
        encode_callable_selection_branches(&branches, slot, 0, output)
    }

    fn encode_callable_selection_branches(
        branches: &[Vec<String>],
        slot: u8,
        selection: usize,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (branch, remaining) = branches.split_first()?;
        if remaining.is_empty() {
            output.extend(branch.iter().cloned());
            return Some(());
        }
        output.push(format!("c{:016x}", (slot as f64).to_bits()));
        output.push("globalget".into());
        output.push(format!("c{:016x}", (selection as f64).to_bits()));
        output.push("==".into());
        output.push("if".into());
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_callable_selection_branches(remaining, slot, selection + 1, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_callable_table_update(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let (table, keys, assigned_helpers) = callable_table_assignment(expression)?;
        let Expr::Assign(assignment) = expression else {
            unreachable!()
        };
        if let Some((table_id, helpers)) = context.dynamic_callable_tables.get(&table).cloned() {
            if !assigned_helpers
                .iter()
                .all(|helper| helpers.contains(helper))
            {
                return None;
            }
            let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
                unreachable!()
            };
            match &target.prop {
                MemberProp::Ident(property) => encode_string(property.sym.as_ref(), output)?,
                MemberProp::Computed(property) => {
                    let mut key = Vec::new();
                    encode_expression(
                        property.expr.as_ref(),
                        parameters,
                        locals,
                        context,
                        &mut key,
                    )?;
                    append_string(key, output)?;
                }
                MemberProp::PrivateName(_) => return None,
            }
            output.push(format!("c{:016x}", (table_id as f64).to_bits()));
            encode_callable_assignment_selection(
                assignment.right.as_ref(),
                &helpers,
                parameters,
                locals,
                context,
                output,
            )?;
            output.extend(["callableset".into(), "drop".into()]);
            return Some(());
        }
        let keys = keys?;
        let updates = keys
            .iter()
            .map(|key| {
                let (slot, helpers) = context.callable_table_states.get(&table)?.get(key)?;
                assigned_helpers
                    .iter()
                    .all(|helper| helpers.contains(helper))
                    .then(|| (key.clone(), *slot, helpers.clone()))
            })
            .collect::<Option<Vec<_>>>()?;
        if let [(_, slot, helpers)] = updates.as_slice() {
            output.push(format!("c{:016x}", (*slot as f64).to_bits()));
            encode_callable_assignment_selection(
                assignment.right.as_ref(),
                helpers,
                parameters,
                locals,
                context,
                output,
            )?;
            output.extend(["globalset".into(), "drop".into()]);
            return Some(());
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            unreachable!()
        };
        let MemberProp::Computed(property) = &target.prop else {
            return None;
        };
        let mut key = Vec::new();
        encode_expression(
            property.expr.as_ref(),
            parameters,
            locals,
            context,
            &mut key,
        )?;
        append_string(key, output)?;
        encode_callable_table_update_branches(
            &updates,
            assignment.right.as_ref(),
            parameters,
            locals,
            context,
            output,
        )?;
        output.extend(["nip".into(), "drop".into()]);
        Some(())
    }

    fn encode_callable_assignment_selection(
        expression: &Expr,
        helpers: &[String],
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match expression {
            Expr::Ident(helper) => {
                let selection = helpers
                    .iter()
                    .position(|candidate| candidate == helper.sym.as_ref())?;
                output.push(format!("c{:016x}", (selection as f64).to_bits()));
            }
            Expr::Paren(parenthesized) => encode_callable_assignment_selection(
                parenthesized.expr.as_ref(),
                helpers,
                parameters,
                locals,
                context,
                output,
            )?,
            Expr::Cond(conditional) => {
                encode_condition(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("if".into());
                encode_callable_assignment_selection(
                    conditional.cons.as_ref(),
                    helpers,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("else".into());
                encode_callable_assignment_selection(
                    conditional.alt.as_ref(),
                    helpers,
                    parameters,
                    locals,
                    context,
                    output,
                )?;
                output.push("end".into());
            }
            _ => return None,
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_callable_table_update_branches(
        updates: &[(String, u8, Vec<String>)],
        assigned: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let ((key, slot, helpers), remaining) = updates.split_first()?;
        if remaining.is_empty() {
            output.push(format!("c{:016x}", (*slot as f64).to_bits()));
            encode_callable_assignment_selection(
                assigned, helpers, parameters, locals, context, output,
            )?;
            output.push("globalset".into());
            return Some(());
        }
        output.push("dup".into());
        encode_string(key, output)?;
        output.extend([
            "strcmp".into(),
            "c0000000000000000".into(),
            "==".into(),
            "if".into(),
            format!("c{:016x}", (*slot as f64).to_bits()),
        ]);
        encode_callable_assignment_selection(
            assigned, helpers, parameters, locals, context, output,
        )?;
        output.extend(["globalset".into(), "else".into()]);
        encode_callable_table_update_branches(
            remaining,
            assigned,
            parameters,
            locals,
            context,
            output,
        )?;
        output.push("end".into());
        Some(())
    }

    fn encode_callable_table_branches(
        branches: &[(String, Vec<String>)],
        missing: &str,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Some(((key, branch), remaining)) = branches.split_first() else {
            output.push(missing.into());
            return Some(());
        };
        output.push("dup".into());
        encode_string(key, output)?;
        output.push("strcmp".into());
        output.push("c0000000000000000".into());
        output.push("==".into());
        output.push("if".into());
        output.extend(branch.iter().cloned());
        output.push("else".into());
        encode_callable_table_branches(remaining, missing, output)?;
        output.push("end".into());
        Some(())
    }

    fn encode_helper_call(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        encode_helper_target(
            callee.as_ref(),
            &call.args,
            parameters,
            locals,
            context,
            output,
        )
    }

    fn encode_recursive_argument(
        expression: &Expr,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<usize> {
        match ty {
            thaw_hir::HirType::Object(fields) => {
                if let Some(object) = object_literal(expression) {
                    if object.props.len() != fields.len() {
                        return None;
                    }
                    let mut slots = 0;
                    for (property, (field, field_type)) in object.props.iter().zip(fields) {
                        let (name, value) = object_property(property)?;
                        let ObjectReturnValue::Expression(value) = value else {
                            return None;
                        };
                        if &name != field {
                            return None;
                        }
                        slots += encode_recursive_argument(
                            value,
                            field_type,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    }
                    return Some(slots);
                }
                encode_recursive_path(&member_path(expression)?, ty, parameters, output)
            }
            thaw_hir::HirType::Tuple(types) => {
                if let Expr::Array(array) = expression {
                    if array.elems.len() != types.len() {
                        return None;
                    }
                    let mut slots = 0;
                    for (element, ty) in array.elems.iter().zip(types) {
                        let element = element.as_ref()?;
                        if element.spread.is_some() {
                            return None;
                        }
                        slots += encode_recursive_argument(
                            element.expr.as_ref(),
                            ty,
                            parameters,
                            locals,
                            context,
                            output,
                        )?;
                    }
                    return Some(slots);
                }
                encode_recursive_path(&member_path(expression)?, ty, parameters, output)
            }
            _ => {
                let expected = jit_return_kind(ty)?;
                let mut encoded = Vec::new();
                encode_expression(
                    expression,
                    parameters,
                    locals,
                    context,
                    &mut encoded,
                )?;
                if jit_expression_kind(&encoded)?.0 != expected {
                    return None;
                }
                output.extend(encoded);
                Some(1)
            }
        }
    }

    fn encode_recursive_path(
        path: &str,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        output: &mut Vec<String>,
    ) -> Option<usize> {
        match ty {
            thaw_hir::HirType::Object(fields) => fields.iter().try_fold(0, |slots, (field, ty)| {
                encode_recursive_path(&format!("{path}.{field}"), ty, parameters, output)
                    .map(|count| slots + count)
            }),
            thaw_hir::HirType::Tuple(types) => {
                types.iter().enumerate().try_fold(0, |slots, (index, ty)| {
                    encode_recursive_path(&format!("{path}.{index}"), ty, parameters, output)
                        .map(|count| slots + count)
                })
            }
            _ => {
                let token = parameters.get(path)?.clone();
                (jit_expression_kind(std::slice::from_ref(&token))?.0 == jit_return_kind(ty)?)
                    .then(|| {
                        output.push(token);
                        1
                    })
            }
        }
    }

    fn resolve_callable<'a>(
        expression: &'a Expr,
        declarations: &std::collections::HashMap<String, NumericCallable<'a>>,
    ) -> Option<NumericCallable<'a>> {
        match expression {
            Expr::Fn(function) => Some(NumericCallable::Function(function.function.as_ref())),
            Expr::Arrow(function) => Some(NumericCallable::Arrow(function)),
            Expr::Ident(identifier) => declarations.get(identifier.sym.as_ref()).copied(),
            _ => None,
        }
    }

    fn resolve_callable_table(
        object: &thaw_parser::ast::ObjectLit,
        declarations: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<Vec<(String, String)>> {
        let mut names = std::collections::HashSet::new();
        let entries = object
            .props
            .iter()
            .map(|property| {
                let PropOrSpread::Prop(property) = property else {
                    return None;
                };
                let (key, value) = match property.as_ref() {
                    Prop::Shorthand(identifier) => {
                        (identifier.sym.to_string(), identifier.sym.to_string())
                    }
                    Prop::KeyValue(property) => {
                        let key = match &property.key {
                            PropName::Ident(identifier) => identifier.sym.to_string(),
                            PropName::Str(string) => {
                                string.value.to_string_lossy().into_owned()
                            }
                            _ => return None,
                        };
                        let Expr::Ident(value) = property.value.as_ref() else {
                            return None;
                        };
                        (key, value.sym.to_string())
                    }
                    _ => return None,
                };
                if !names.insert(key.clone()) || !declarations.contains_key(&value) {
                    return None;
                }
                Some((key, value))
            })
            .collect::<Option<Vec<_>>>()?;
        (!entries.is_empty()).then_some(entries)
    }

    fn finite_string_keys(expression: &Expr) -> Option<Vec<String>> {
        match expression {
            Expr::Lit(Lit::Str(key)) => Some(vec![key.value.to_string_lossy().into_owned()]),
            Expr::Paren(parenthesized) => finite_string_keys(parenthesized.expr.as_ref()),
            Expr::Cond(conditional) => {
                let mut keys = finite_string_keys(conditional.cons.as_ref())?;
                keys.extend(finite_string_keys(conditional.alt.as_ref())?);
                keys.sort();
                keys.dedup();
                Some(keys)
            }
            _ => None,
        }
    }

    fn encoded_string_literal(tokens: &[String]) -> Option<String> {
        let [token] = tokens else {
            return None;
        };
        let encoded = token.strip_prefix('t')?;
        if encoded.len() % 2 != 0 {
            return None;
        }
        let bytes = (0..encoded.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
            .collect::<Option<Vec<_>>>()?;
        String::from_utf8(bytes).ok()
    }

    fn finite_callable_names(expression: &Expr) -> Option<Vec<String>> {
        match expression {
            Expr::Ident(helper) => Some(vec![helper.sym.to_string()]),
            Expr::Paren(parenthesized) => finite_callable_names(parenthesized.expr.as_ref()),
            Expr::Cond(conditional) => {
                let mut helpers = finite_callable_names(conditional.cons.as_ref())?;
                helpers.extend(finite_callable_names(conditional.alt.as_ref())?);
                helpers.sort();
                helpers.dedup();
                Some(helpers)
            }
            _ => None,
        }
    }

    fn callable_member_helpers(
        expression: &Expr,
        context: &InlineContext<'_>,
    ) -> Option<Vec<String>> {
        let expression = match expression {
            Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
            expression => expression,
        };
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(table) = member.obj.as_ref() else {
            return None;
        };
        if let Some((_, helpers)) = context
            .dynamic_callable_tables
            .get(table.sym.as_ref())
        {
            return Some(helpers.clone());
        }
        let key = match &member.prop {
            MemberProp::Ident(property) => property.sym.as_ref(),
            MemberProp::Computed(property) => {
                let Expr::Lit(Lit::Str(key)) = property.expr.as_ref() else {
                    return None;
                };
                key.value.as_str()?
            }
            MemberProp::PrivateName(_) => return None,
        };
        context
            .callable_table_states
            .get(table.sym.as_ref())?
            .get(key)
            .map(|(_, helpers)| helpers.clone())
    }

    type CallableTableAssignment = (String, Option<Vec<String>>, Vec<String>);

    fn callable_table_assignment(
        expression: &Expr,
    ) -> Option<CallableTableAssignment> {
        let Expr::Assign(assignment) = expression else {
            return None;
        };
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            return None;
        };
        let Expr::Ident(table) = target.obj.as_ref() else {
            return None;
        };
        let keys = match &target.prop {
            MemberProp::Ident(property) => Some(vec![property.sym.to_string()]),
            MemberProp::Computed(property) => finite_string_keys(property.expr.as_ref()),
            MemberProp::PrivateName(_) => return None,
        };
        let helpers = finite_callable_names(assignment.right.as_ref())?;
        Some((table.sym.to_string(), keys, helpers))
    }

    fn collect_callable_table_assignments(
        statement: &Stmt,
        output: &mut Vec<CallableTableAssignment>,
    ) {
        match statement {
            Stmt::Expr(statement) => {
                if let Some(assignment) = callable_table_assignment(statement.expr.as_ref()) {
                    output.push(assignment);
                }
            }
            Stmt::Block(block) => {
                for statement in &block.stmts {
                    collect_callable_table_assignments(statement, output);
                }
            }
            Stmt::If(statement) => {
                collect_callable_table_assignments(statement.cons.as_ref(), output);
                if let Some(alternate) = statement.alt.as_deref() {
                    collect_callable_table_assignments(alternate, output);
                }
            }
            Stmt::While(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::DoWhile(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::For(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::ForIn(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::ForOf(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            Stmt::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.cons {
                        collect_callable_table_assignments(statement, output);
                    }
                }
            }
            Stmt::Try(statement) => {
                for statement in &statement.block.stmts {
                    collect_callable_table_assignments(statement, output);
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.stmts {
                        collect_callable_table_assignments(statement, output);
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.stmts {
                        collect_callable_table_assignments(statement, output);
                    }
                }
            }
            Stmt::Labeled(statement) => {
                collect_callable_table_assignments(statement.body.as_ref(), output);
            }
            _ => {}
        }
    }

    fn exported_callable<'a>(
        assignment: &'a AssignExpr,
        export_name: &str,
        allow_default: bool,
        declarations: &std::collections::HashMap<String, NumericCallable<'a>>,
    ) -> Option<(ExportStyle, Option<NumericCallable<'a>>)> {
        if assignment.op != AssignOp::Assign {
            return None;
        }
        let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
            return None;
        };
        let is_module_exports = matches!(target.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
            && matches!(&target.prop, MemberProp::Ident(property) if property.sym == "exports");
        if is_module_exports {
            if let Expr::Object(object) = assignment.right.as_ref() {
                let mut selected = None;
                for property in &object.props {
                    let PropOrSpread::Prop(property) = property else {
                        return None;
                    };
                    let (is_target, callable) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let is_target = match &property.key {
                                PropName::Ident(identifier) => identifier.sym == export_name,
                                PropName::Str(string) => {
                                    string.value.to_string_lossy() == export_name
                                }
                                _ => return None,
                            };
                            (
                                is_target,
                                resolve_callable(property.value.as_ref(), declarations)?,
                            )
                        }
                        Prop::Shorthand(identifier) => (
                            identifier.sym == export_name,
                            declarations
                                .get(identifier.sym.as_ref())
                                .copied()?,
                        ),
                        _ => return None,
                    };
                    if is_target && selected.replace(callable).is_some() {
                        return None;
                    }
                }
                return Some((ExportStyle::Whole, selected));
            }
            let callable = resolve_callable(assignment.right.as_ref(), declarations)?;
            return Some((
                ExportStyle::Whole,
                allow_default.then_some(callable),
            ));
        }
        let name = if let Expr::Member(object) = target.obj.as_ref() {
            if !matches!(object.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
                || !matches!(&object.prop, MemberProp::Ident(property) if property.sym == "exports")
            {
                return None;
            }
            match &target.prop {
                MemberProp::Ident(property) => property.sym.as_ref(),
                _ => return None,
            }
        } else if matches!(target.obj.as_ref(), Expr::Ident(exports) if exports.sym == "exports") {
            match &target.prop {
                MemberProp::Ident(property) => property.sym.as_ref(),
                _ => return None,
            }
        } else {
            return None;
        };
        let callable = resolve_callable(assignment.right.as_ref(), declarations)?;
        Some((
            ExportStyle::Named,
            (name == export_name).then_some(callable),
        ))
    }

    if function.generic.is_some()
        || function.rest_param.is_some()
        || !function
            .params
            .iter()
            .all(|(_, ty)| {
                let thaw_bridge::DtsType::Native(ty) = ty else {
                    return false;
                };
                let ty = match ty {
                    thaw_hir::HirType::Optional(payload) => payload.as_ref(),
                    ty => ty,
                };
                jit_parameter_slots(ty).is_some()
            })
        || function
            .params
            .iter()
            .try_fold(0usize, |slots, (_, ty)| match ty {
                thaw_bridge::DtsType::Native(ty) => {
                    jit_parameter_slots(ty).map(|count| slots + count)
                }
                thaw_bridge::DtsType::Unsupported(_) => None,
            })
            .is_none_or(|slots| slots > 16)
        || !match &function.ret {
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str,
            ) => true,
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements)) => {
                jit_tagged_union(elements)
            }
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload),
            ) => {
                jit_result_supported(payload)
                    && matches!(
                        payload.as_ref(),
                        thaw_hir::HirType::F64
                            | thaw_hir::HirType::Bool
                            | thaw_hir::HirType::Str
                            | thaw_hir::HirType::Union(_)
                            | thaw_hir::HirType::Array(_)
                            | thaw_hir::HirType::Dictionary(_)
                            | thaw_hir::HirType::Object(_)
                            | thaw_hir::HirType::Tuple(_)
                    )
            }
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element)) => {
                jit_array_result_element_supported(element)
            }
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(element)) => matches!(
                element.as_ref(),
                thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str
            ),
            thaw_bridge::DtsType::Native(
                ty @ (thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)),
            ) => jit_result_supported(ty),
            _ => false,
        }
    {
        return None;
    }

    let module = thaw_parser::parse_javascript(source).ok()?;
    let mut module_functions = std::collections::HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) = item {
            if module_functions
                .insert(
                    declaration.ident.sym.to_string(),
                    NumericCallable::Function(declaration.function.as_ref()),
                )
                .is_some()
            {
                return None;
            }
        }
    }
    if module_functions.contains_key("String")
        || module_functions.contains_key("Number")
        || module_functions.contains_key("Boolean")
    {
        return None;
    }
    let mut style = None;
    let mut callable = None;
    let mut module_locals = std::collections::HashMap::new();
    let mut module_mutable = std::collections::HashSet::new();
    let mut module_callable_mutable = std::collections::HashSet::new();
    let mut module_callable_tables = std::collections::HashMap::new();
    let mut module_callable_table_mutable = std::collections::HashSet::new();
    let no_parameters = std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(statement) = item else {
            return None;
        };
        if matches!(statement, Stmt::Decl(Decl::Fn(_))) {
            continue;
        }
        if let Stmt::Decl(Decl::Var(declaration)) = statement {
            for declarator in &declaration.decls {
                let Pat::Ident(name) = &declarator.name else {
                    return None;
                };
                if module_locals.contains_key(name.id.sym.as_ref())
                    || module_functions.contains_key(name.id.sym.as_ref())
                    || module_callable_tables.contains_key(name.id.sym.as_ref())
                {
                    return None;
                }
                let initializer = declarator.init.as_deref()?;
                if let Expr::Object(object) = initializer {
                    if let Some(table) = resolve_callable_table(object, &module_functions) {
                        module_callable_tables.insert(name.id.sym.to_string(), table);
                        if declaration.kind != VarDeclKind::Const {
                            module_callable_table_mutable.insert(name.id.sym.to_string());
                        }
                        continue;
                    }
                }
                if let Some(callable) = resolve_callable(initializer, &module_functions) {
                    module_functions.insert(name.id.sym.to_string(), callable);
                    if declaration.kind != VarDeclKind::Const {
                        module_callable_mutable.insert(name.id.sym.to_string());
                    }
                    continue;
                }
                let mut encoded = Vec::new();
                let empty_helpers = std::collections::HashMap::new();
                let mut context = InlineContext {
                    helpers: &empty_helpers,
                    module_locals: module_locals.clone(),
                    module_globals: std::collections::HashMap::new(),
                    callable_tables: module_callable_tables.clone(),
                    callable_table_states: std::collections::HashMap::new(),
                    dynamic_callable_tables: std::collections::HashMap::new(),
                    active: Vec::new(),
                    recursive_names: Vec::new(),
                    recursive_parameters: Vec::new(),
                    recursive_result: None,
                };
                encode_expression(
                    initializer,
                    &no_parameters,
                    &module_locals,
                    &mut context,
                    &mut encoded,
                )?;
                if declaration.kind != VarDeclKind::Const && !stable_jit_tokens(&encoded) {
                    return None;
                }
                module_locals.insert(name.id.sym.to_string(), encoded);
                if declaration.kind != VarDeclKind::Const {
                    module_mutable.insert(name.id.sym.to_string());
                }
            }
            continue;
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        if matches!(statement.expr.as_ref(), Expr::Lit(Lit::Str(_))) {
            continue;
        }
        let Expr::Assign(assignment) = statement.expr.as_ref() else {
            if let Expr::Update(update) = statement.expr.as_ref() {
                if let Expr::Ident(name) = update.arg.as_ref() {
                    if module_mutable.contains(name.sym.as_ref()) {
                        let mut encoded = module_locals.get(name.sym.as_ref())?.clone();
                        encoded.extend([
                            format!("c{:016x}", 1.0_f64.to_bits()),
                            match update.op {
                                UpdateOp::PlusPlus => "+".into(),
                                UpdateOp::MinusMinus => "-".into(),
                            },
                        ]);
                        module_locals.insert(name.sym.to_string(), encoded);
                        continue;
                    }
                }
            }
            return None;
        };
        if let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left {
            if module_callable_table_mutable.contains(name.id.sym.as_ref()) {
                if assignment.op != AssignOp::Assign {
                    return None;
                }
                let Expr::Object(object) = assignment.right.as_ref() else {
                    return None;
                };
                let table = resolve_callable_table(object, &module_functions)?;
                module_callable_tables.insert(name.id.sym.to_string(), table);
                continue;
            }
            if module_callable_mutable.contains(name.id.sym.as_ref()) {
                if assignment.op != AssignOp::Assign {
                    return None;
                }
                let callable = resolve_callable(assignment.right.as_ref(), &module_functions)?;
                module_functions.insert(name.id.sym.to_string(), callable);
                continue;
            }
            if !module_mutable.contains(name.id.sym.as_ref()) {
                return None;
            }
            let mut encoded = Vec::new();
            if assignment.op == AssignOp::AddAssign {
                let mut right = Vec::new();
                encode_expression(
                    assignment.right.as_ref(),
                    &no_parameters,
                    &module_locals,
                    &mut InlineContext {
                        helpers: &module_functions,
                        module_locals: module_locals.clone(),
                        module_globals: std::collections::HashMap::new(),
                        callable_tables: module_callable_tables.clone(),
                        callable_table_states: std::collections::HashMap::new(),
                        dynamic_callable_tables: std::collections::HashMap::new(),
                        active: Vec::new(),
                        recursive_names: Vec::new(),
                        recursive_parameters: Vec::new(),
                        recursive_result: None,
                    },
                    &mut right,
                )?;
                append_add(
                    module_locals.get(name.id.sym.as_ref())?.clone(),
                    right,
                    &mut encoded,
                )?;
            } else {
                if assignment.op != AssignOp::Assign {
                    encoded.extend(
                        module_locals
                            .get(name.id.sym.as_ref())?
                            .iter()
                            .cloned(),
                    );
                }
                let mut context = InlineContext {
                    helpers: &module_functions,
                    module_locals: module_locals.clone(),
                    module_globals: std::collections::HashMap::new(),
                    callable_tables: module_callable_tables.clone(),
                    callable_table_states: std::collections::HashMap::new(),
                    dynamic_callable_tables: std::collections::HashMap::new(),
                    active: Vec::new(),
                    recursive_names: Vec::new(),
                    recursive_parameters: Vec::new(),
                    recursive_result: None,
                };
                encode_expression(
                    assignment.right.as_ref(),
                    &no_parameters,
                    &module_locals,
                    &mut context,
                    &mut encoded,
                )?;
                if assignment.op != AssignOp::Assign {
                    encoded.push(
                        match assignment.op {
                            AssignOp::SubAssign => "-",
                            AssignOp::MulAssign => "*",
                            AssignOp::DivAssign => "/",
                            AssignOp::ModAssign => "%",
                            AssignOp::LShiftAssign => "shl",
                            AssignOp::RShiftAssign => "shr",
                            AssignOp::ZeroFillRShiftAssign => "ushr",
                            AssignOp::BitOrAssign => "bor",
                            AssignOp::BitXorAssign => "bxor",
                            AssignOp::BitAndAssign => "band",
                            AssignOp::ExpAssign => "pow",
                            _ => return None,
                        }
                        .into(),
                    );
                }
            }
            if !stable_jit_tokens(&encoded) {
                return None;
            }
            module_locals.insert(name.id.sym.to_string(), encoded);
            continue;
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left {
            if let Expr::Ident(table) = target.obj.as_ref() {
                if let Some(entries) = module_callable_tables.get_mut(table.sym.as_ref()) {
                    if assignment.op != AssignOp::Assign {
                        return None;
                    }
                    let key = match &target.prop {
                        MemberProp::Ident(property) => property.sym.to_string(),
                        MemberProp::Computed(property) => {
                            match property.expr.as_ref() {
                                Expr::Lit(Lit::Str(key)) => {
                                    key.value.to_string_lossy().into_owned()
                                }
                                Expr::Ident(key) => encoded_string_literal(
                                    module_locals.get(key.sym.as_ref())?,
                                )?,
                                _ => return None,
                            }
                        }
                        MemberProp::PrivateName(_) => return None,
                    };
                    let helper = match assignment.right.as_ref() {
                        Expr::Ident(helper) if module_functions.contains_key(helper.sym.as_ref()) => {
                            helper.sym.to_string()
                        }
                        _ => return None,
                    };
                    if let Some((_, value)) = entries.iter_mut().find(|(name, _)| name == &key) {
                        *value = helper;
                    } else {
                        entries.push((key, helper));
                    }
                    continue;
                }
            }
        }
        let (assignment_style, selected) =
            exported_callable(assignment, export_name, allow_default, &module_functions)?;
        if style.replace(assignment_style).is_some_and(|style| {
            style != assignment_style || assignment_style == ExportStyle::Whole
        }) {
            return None;
        }
        if let Some(selected) = selected {
            if callable.replace(selected).is_some() {
                return None;
            }
        }
    }
    let callable = callable?;

    let (params, local_steps, body) = callable_parts(callable)?;
    if params.len() != function.params.len() {
        return None;
    }
    let mut bindings = Vec::with_capacity(params.len());
    let mut names = std::collections::HashSet::new();
    for (index, (parameter, (_, ty))) in params.iter().zip(&function.params).enumerate() {
        let (parameter, default) = match parameter {
            Pat::Ident(parameter) => (parameter, None),
            Pat::Assign(assignment) => {
                let Pat::Ident(parameter) = assignment.left.as_ref() else {
                    return None;
                };
                (parameter, Some(assignment.right.as_ref()))
            }
            _ => return None,
        };
        let name = parameter.id.sym.to_string();
        if !names.insert(name.clone()) {
            return None;
        }
        let thaw_bridge::DtsType::Native(ty) = ty else {
            return None;
        };
        bindings.push((name, ty, default, index >= function.required_params));
    }
    let mut expression = Vec::new();
    let mut module_globals = std::collections::HashMap::new();
    let stateful_module_names = module_functions
        .values()
        .filter_map(|callable| callable_parts(*callable))
        .flat_map(|(_, steps, _)| steps)
        .filter_map(|step| match step {
            LocalStep::Assign { name, .. } | LocalStep::Update { name, .. }
                if module_mutable.contains(name.sym.as_ref()) =>
            {
                Some(name.sym.to_string())
            }
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let mut global_names = stateful_module_names
        .iter()
        .filter_map(|name| {
            let initial = module_locals.get(name)?;
            (jit_expression_kind(initial)?.0 == JitKind::Number).then(|| name.clone())
        })
        .collect::<Vec<_>>();
    global_names.sort();
    global_names.dedup();
    if global_names.len() > 16 {
        return None;
    }
    let mut next_global_slot = global_names.len();
    for (slot, name) in global_names.into_iter().enumerate() {
        let initial = module_locals.get(&name)?.clone();
        let mut read = vec![format!("c{:016x}", (slot as f64).to_bits())];
        read.extend(initial);
        read.push("globalinit".into());
        module_locals.insert(name.clone(), read);
        module_globals.insert(name, slot as u8);
    }
    let mut table_candidates = std::collections::BTreeMap::<
        (String, String),
        std::collections::BTreeSet<String>,
    >::new();
    let mut assignments = Vec::new();
    for (_, steps, body) in module_functions
        .values()
        .filter_map(|callable| callable_parts(*callable))
    {
        for step in steps {
            if let LocalStep::Effect(expression) = step {
                if let Some(assignment) = callable_table_assignment(expression) {
                    assignments.push(assignment);
                }
            }
        }
        if let NumericBody::Statements(statements) = body {
            for statement in statements {
                collect_callable_table_assignments(statement, &mut assignments);
            }
        }
    }
    let dynamic_table_names = assignments
        .iter()
        .filter_map(|(table, keys, _)| keys.is_none().then_some(table.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut dynamic_table_candidates = std::collections::BTreeMap::<
        String,
        std::collections::BTreeSet<String>,
    >::new();
    for (table, keys, helpers) in assignments {
        if helpers
            .iter()
            .any(|helper| !module_functions.contains_key(helper))
            || !module_callable_tables.contains_key(&table)
        {
            continue;
        }
        if dynamic_table_names.contains(&table) {
            dynamic_table_candidates
                .entry(table)
                .or_default()
                .extend(helpers);
            continue;
        }
        for key in keys? {
                if !module_callable_tables
                    .get(&table)
                    .is_some_and(|entries| entries.iter().any(|(name, _)| name == &key))
                {
                    continue;
                }
                table_candidates
                    .entry((table.clone(), key))
                    .or_default()
                    .extend(helpers.iter().cloned());
        }
    }
    for table in &dynamic_table_names {
        let Some(entries) = module_callable_tables.get(table) else {
            continue;
        };
        let helpers = entries.iter().map(|(_, helper)| helper.clone());
        dynamic_table_candidates
            .entry(table.clone())
            .or_default()
            .extend(helpers);
    }
    if next_global_slot + table_candidates.len() > 16 {
        return None;
    }
    let mut callable_table_states = std::collections::HashMap::<
        String,
        std::collections::HashMap<String, (u8, Vec<String>)>,
    >::new();
    for ((table, key), candidates) in table_candidates {
        let initial = module_callable_tables
            .get(&table)?
            .iter()
            .find(|(name, _)| name == &key)?
            .1
            .clone();
        let mut helpers = vec![initial.clone()];
        helpers.extend(candidates.into_iter().filter(|helper| helper != &initial));
        callable_table_states
            .entry(table)
            .or_default()
            .insert(key, (next_global_slot as u8, helpers));
        next_global_slot += 1;
    }
    let dynamic_callable_tables = dynamic_table_candidates
        .into_iter()
        .enumerate()
        .map(|(table, (name, helpers))| {
            Some((
                name,
                (u8::try_from(table).ok()?, helpers.into_iter().collect()),
            ))
        })
        .collect::<Option<std::collections::HashMap<_, _>>>()?;
    let helper_module_locals = module_locals.clone();
    let helper_callable_tables = module_callable_tables.clone();
    let mut locals = module_locals;
    for (parameter, _, _, _) in &bindings {
        locals.remove(parameter);
    }
    let recursive_parameters = bindings
        .iter()
        .try_fold((0usize, Vec::new()), |(slots, mut types), (_, ty, default, optional)| {
            if default.is_some() || *optional {
                return None;
            }
            let count = jit_parameter_slots(ty)?;
            if slots + count > 8 {
                return None;
            }
            types.push((*ty).clone());
            Some((slots + count, types))
        })
        .map(|(_, types)| types)
        .unwrap_or_default();
    let recursive_result = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::F64) => Some(JitKind::Number),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool) => Some(JitKind::Boolean),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Str) => Some(JitKind::String),
        _ => None,
    };
    let recursive_names = if !params.is_empty()
        && recursive_parameters.len() == params.len()
        && recursive_result.is_some()
    {
        module_functions
            .iter()
            .filter(|(_, candidate)| same_callable(**candidate, callable))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut context = InlineContext {
        helpers: &module_functions,
        module_locals: helper_module_locals,
        module_globals,
        callable_tables: helper_callable_tables,
        callable_table_states,
        dynamic_callable_tables,
        active: Vec::new(),
        recursive_names,
        recursive_parameters,
        recursive_result,
    };
    let mut parameters = std::collections::HashMap::new();
    let mut slot = 0usize;
    for (parameter, ty, default, optional_parameter) in &bindings {
        if let thaw_hir::HirType::Nullable(payload) | thaw_hir::HirType::Nullish(payload) = ty {
            if default.is_some() || *optional_parameter {
                return None;
            }
            if matches!(
                payload.as_ref(),
                thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)
            ) {
                let presence = if matches!(ty, thaw_hir::HirType::Nullable(_)) {
                    format!("a{slot}")
                } else {
                    format!("a{slot},c0000000000000000,==")
                };
                slot += 1;
                bind_jit_aggregate_fields(parameter, payload, &mut parameters, &mut slot)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:aggregate"),
                );
                continue;
            }
            let (prefix, null, absent) = match payload.as_ref() {
                thaw_hir::HirType::F64 => ("a", "nulln", "absentn"),
                thaw_hir::HirType::Bool => ("b", "nullb", "absentb"),
                thaw_hir::HirType::Str => ("s", "nulls", "absents"),
                thaw_hir::HirType::Array(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => ("rn", "nulla", "absenta"),
                    thaw_hir::HirType::Bool => ("rb", "nulla", "absenta"),
                    thaw_hir::HirType::Str => ("rs", "nulla", "absenta"),
                    _ => return None,
                },
                thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => ("dn", "nulld", "absentd"),
                    thaw_hir::HirType::Bool => ("db", "nulld", "absentd"),
                    thaw_hir::HirType::Str => ("ds", "nulld", "absentd"),
                    _ => return None,
                },
                _ => return None,
            };
            let tag = format!("a{slot}");
            let value = format!("{prefix}{}", slot + 1);
            let selected = if matches!(ty, thaw_hir::HirType::Nullable(_)) {
                vec![
                    tag,
                    "asbool".into(),
                    "if".into(),
                    value,
                    "else".into(),
                    null.into(),
                    "end".into(),
                ]
            } else {
                vec![
                    tag.clone(),
                    "c0000000000000000".into(),
                    "==".into(),
                    "if".into(),
                    value,
                    "else".into(),
                    tag,
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "==".into(),
                    "if".into(),
                    null.into(),
                    "else".into(),
                    absent.into(),
                    "end".into(),
                    "end".into(),
                ]
            };
            jit_expression_kind(&selected)?;
            locals.insert(parameter.clone(), selected);
            slot += 2;
            continue;
        }
        let (ty, optional_type) = match ty {
            thaw_hir::HirType::Optional(payload) => (payload.as_ref(), true),
            ty => (*ty, false),
        };
        let optional = *optional_parameter || optional_type;
        if let thaw_hir::HirType::Union(elements) = ty {
            if !jit_argument_tagged_union(elements) {
                return None;
            }
            let kinds = elements
                .iter()
                .map(jit_union_member_code)
                .collect::<Option<String>>()?;
            if let Some(default) = default {
                if !optional {
                    return None;
                }
                if elements
                    .iter()
                    .any(|element| matches!(element, thaw_hir::HirType::Object(_)))
                {
                    return None;
                }
                let mut present = vec![format!("u{}{kinds}", slot + 1)];
                let mut fallback = Vec::new();
                encode_expression(
                    default,
                    &parameters,
                    &locals,
                    &mut context,
                    &mut fallback,
                )?;
                normalize_callable_branches([&mut present, &mut fallback])?;
                let mut selected = vec![format!("a{slot}"), "asbool".into(), "if".into()];
                selected.extend(present);
                selected.push("else".into());
                selected.extend(fallback);
                selected.push("end".into());
                locals.insert(parameter.clone(), selected);
                slot += 3;
                continue;
            }
            if optional {
                let source = vec![format!("u{}{kinds}", slot + 1)];
                bind_jit_union_object_fields(parameter, elements, &source, &mut locals)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:a{slot}:u{}{kinds}", slot + 1),
                );
                slot += 3;
                continue;
            }
            let source = vec![format!("u{slot}{kinds}")];
            bind_jit_union_object_fields(parameter, elements, &source, &mut locals)?;
            locals.insert(parameter.clone(), source);
            slot += 2;
            continue;
        }
        if matches!(
            ty,
            thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)
        ) {
            if default.is_some() {
                return None;
            }
            if optional {
                let presence = format!("a{slot}");
                slot += 1;
                bind_jit_aggregate_fields(parameter, ty, &mut parameters, &mut slot)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:aggregate"),
                );
                continue;
            }
            bind_jit_aggregate_fields(parameter, ty, &mut parameters, &mut slot)?;
            continue;
        }
        let prefix = match ty {
            thaw_hir::HirType::Str => "s",
            thaw_hir::HirType::Bool => "b",
            thaw_hir::HirType::Array(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => "rn",
                thaw_hir::HirType::Bool => "rb",
                thaw_hir::HirType::Str => "rs",
                thaw_hir::HirType::Tuple(elements) => match elements.as_slice() {
                    [thaw_hir::HirType::Str, thaw_hir::HirType::F64] => "en",
                    [thaw_hir::HirType::Str, thaw_hir::HirType::Bool] => "eb",
                    [thaw_hir::HirType::Str, thaw_hir::HirType::Str] => "es",
                    _ => return None,
                },
                _ => return None,
            },
            thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => "dn",
                thaw_hir::HirType::Bool => "db",
                thaw_hir::HirType::Str => "ds",
                _ => return None,
            },
            _ => "a",
        };
        if optional {
            let presence = format!("a{slot}");
            let value = format!("{prefix}{}", slot + 1);
            let Some(default) = default.as_ref() else {
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:{value}"),
                );
                slot += 2;
                continue;
            };
            let mut fallback = Vec::new();
            encode_expression(
                default,
                &parameters,
                &locals,
                &mut context,
                &mut fallback,
            )?;
            if *ty == thaw_hir::HirType::Bool {
                let mut boolean = Vec::new();
                append_boolean(fallback, &mut boolean)?;
                fallback = boolean;
            }
            let mut selected = vec![presence, "asbool".into(), "if".into(), value, "else".into()];
            selected.extend(fallback);
            selected.push("end".into());
            jit_expression_kind(&selected)?;
            locals.insert(parameter.clone(), selected);
            slot += 2;
        } else {
            parameters.insert(parameter.clone(), format!("{prefix}{slot}"));
            slot += 1;
        }
    }
    if slot > 16 {
        return None;
    }
    let starts_loop = matches!(body, NumericBody::Statements(statements)
        if matches!(statements.first(), Some(Stmt::While(_) | Stmt::For(_) | Stmt::DoWhile(_) | Stmt::ForIn(_) | Stmt::ForOf(_))));
    if !starts_loop
        && matches!(
            &function.ret,
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::Object(_)
                    | thaw_hir::HirType::Dictionary(_)
                    | thaw_hir::HirType::Tuple(_)
            )
        )
    {
        let thaw_bridge::DtsType::Native(ty) = &function.ret else {
            unreachable!()
        };
        let mut jit_locals = Vec::new();
        let mut mutable = std::collections::HashSet::new();
        for step in local_steps {
            if slot == 16 {
                return None;
            }
            let mut encoded = Vec::new();
            let binding = match step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: is_mutable,
                } => {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    encode_expression(
                        initializer,
                        &parameters,
                        &locals,
                        &mut context,
                        &mut encoded,
                    )?;
                    if is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    Some(name)
                }
                LocalStep::Assign {
                    name,
                    operation,
                    value,
                } => {
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    if operation == AssignOp::AddAssign {
                        let mut right = Vec::new();
                        encode_expression(
                            value,
                            &parameters,
                            &locals,
                            &mut context,
                            &mut right,
                        )?;
                        append_add(
                            locals.get(name.sym.as_ref())?.clone(),
                            right,
                            &mut encoded,
                        )?;
                    } else {
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(
                            value,
                            &parameters,
                            &locals,
                            &mut context,
                            &mut encoded,
                        )?;
                        if operation != AssignOp::Assign {
                            encoded.push(
                                match operation {
                                    AssignOp::SubAssign => "-",
                                    AssignOp::MulAssign => "*",
                                    AssignOp::DivAssign => "/",
                                    AssignOp::ModAssign => "%",
                                    AssignOp::LShiftAssign => "shl",
                                    AssignOp::RShiftAssign => "shr",
                                    AssignOp::ZeroFillRShiftAssign => "ushr",
                                    AssignOp::BitOrAssign => "bor",
                                    AssignOp::BitXorAssign => "bxor",
                                    AssignOp::BitAndAssign => "band",
                                    AssignOp::ExpAssign => "pow",
                                    _ => return None,
                                }
                                .into(),
                            );
                        }
                    }
                    Some(name)
                }
                LocalStep::Update { name, operation } => {
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                    encoded.push(format!("c{:016x}", 1.0f64.to_bits()));
                    encoded.push(
                        match operation {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    Some(name)
                }
                LocalStep::Effect(expression) => {
                    encode_expression(
                        expression,
                        &parameters,
                        &locals,
                        &mut context,
                        &mut encoded,
                    )?;
                    None
                }
            };
            let kind = jit_expression_kind(&encoded)?.0;
            let (ty, prefix) = match kind {
                JitKind::Number => (thaw_hir::HirType::F64, "a"),
                JitKind::Boolean => (thaw_hir::HirType::Bool, "b"),
                JitKind::String => (thaw_hir::HirType::Str, "s"),
                JitKind::Dynamic => (
                    thaw_hir::HirType::Union(vec![
                        thaw_hir::HirType::F64,
                        thaw_hir::HirType::Bool,
                        thaw_hir::HirType::Str,
                    ]),
                    "ld",
                ),
                JitKind::Array => match array_prefix(&encoded)? {
                    "rn" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
                        "rn",
                    ),
                    "rb" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool)),
                        "rb",
                    ),
                    "rs" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str)),
                        "rs",
                    ),
                    _ => return None,
                },
                JitKind::Dictionary => return None,
            };
            let operation = validated_jit_expression(encoded, kind)?;
            if let Some(name) = binding {
                locals.insert(name.sym.to_string(), vec![format!("{prefix}{slot}")]);
            }
            jit_locals.push(JitLocal { ty, operation });
            slot += 1;
        }
        let result = encode_aggregate_body(
            body,
            ty,
            &parameters,
            &locals,
            &mut context,
        )?;
        return Some(if jit_locals.is_empty() {
            result
        } else {
            JitExport::WithLocals(jit_locals, Box::new(result))
        });
    }
    if local_steps.is_empty() && !starts_loop {
        if let thaw_bridge::DtsType::Native(ty) = &function.ret {
            let aggregate_wrapper = matches!(
                ty,
                thaw_hir::HirType::Optional(payload)
                    | thaw_hir::HirType::Nullable(payload)
                    | thaw_hir::HirType::Nullish(payload)
                    if matches!(payload.as_ref(), thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_))
            );
            if jit_result_supported(ty)
                && (aggregate_wrapper
                    || !matches!(
                        ty,
                        thaw_hir::HirType::Optional(_)
                            | thaw_hir::HirType::Nullable(_)
                            | thaw_hir::HirType::Nullish(_)
                    ))
            {
                return encode_aggregate_body(body, ty, &parameters, &locals, &mut context);
            }
        }
    }
    encode_steps_and_body(
        local_steps,
        body,
        &parameters,
        locals,
        &mut context,
        &mut expression,
    )?;
    let expected = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Str) => JitKind::String,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements))
            if jit_tagged_union(elements) =>
        {
            JitKind::Dynamic
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool) => JitKind::Boolean,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if matches!(element.as_ref(), thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str) => JitKind::Array,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(element))
            if matches!(element.as_ref(), thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str) => JitKind::Dictionary,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload),
        ) => jit_return_kind(payload)?,
        _ => JitKind::Number,
    };
    validated_jit_expression(expression, expected).map(JitExport::Value)
}

#[cfg(test)]
fn jit_numeric_export(
    source: &str,
    export_name: &str,
    allow_default: bool,
    function: &thaw_bridge::DtsFunction,
) -> Option<String> {
    fn tokens(value: JitExport) -> Option<Vec<String>> {
        match value {
            JitExport::Value(operation) => operation
                .strip_prefix("expr:")?
                .split(',')
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into(),
            JitExport::Conditional(condition, consequent, alternate) => {
                let mut result = tokens(JitExport::Value(condition))?;
                result.extend(tokens(*consequent)?);
                result.extend(tokens(*alternate)?);
                result.push("?".into());
                Some(result)
            }
            JitExport::Object(_)
            | JitExport::Null
            | JitExport::Undefined
            | JitExport::Dictionary(_)
            | JitExport::Tuple(_)
            | JitExport::WithLocals(_, _) => None,
        }
    }
    let export = jit_export(source, export_name, allow_default, function)?;
    if matches!(
        export,
        JitExport::Object(_)
            | JitExport::Null
            | JitExport::Undefined
            | JitExport::Dictionary(_)
            | JitExport::Tuple(_)
            | JitExport::WithLocals(_, _)
    ) {
        return Some("aggregate".into());
    }
    Some(format!("expr:{}", tokens(export)?.join(",")))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum JitKind {
    Number,
    Boolean,
    String,
    Dynamic,
    Array,
    Dictionary,
}

fn jit_dynamic_argument(token: &str) -> Option<(usize, &str)> {
    let encoded = token.strip_prefix('u')?;
    let digits = encoded.bytes().take_while(u8::is_ascii_digit).count();
    let (index, kinds) = encoded.split_at(digits);
    if kinds.is_empty()
        || !kinds
            .bytes()
            .all(|kind| matches!(kind, b'n' | b'b' | b's' | b'N' | b'B' | b'S' | b'D' | b'E' | b'F' | b'O' | b'T' | b'X' | b'Y' | b'Z'))
    {
        return None;
    }
    Some((index.parse().ok()?, kinds))
}

fn jit_dynamic_array_argument(token: &str) -> bool {
    jit_dynamic_argument(token).is_some_and(|(_, kinds)| {
        kinds.bytes().all(|kind| matches!(kind, b'N' | b'B' | b'S'))
    })
}

fn merge_jit_kinds(left: JitKind, right: JitKind) -> Option<JitKind> {
    if left == right {
        Some(left)
    } else if matches!(left, JitKind::Number | JitKind::Boolean)
        && matches!(right, JitKind::Number | JitKind::Boolean)
    {
        Some(JitKind::Boolean)
    } else if matches!(
        (left, right),
        (JitKind::Number, JitKind::String)
            | (JitKind::String, JitKind::Number)
            | (JitKind::String, JitKind::Boolean)
            | (JitKind::Boolean, JitKind::String)
            | (JitKind::Dynamic, JitKind::Number | JitKind::Boolean | JitKind::String)
            | (JitKind::Number | JitKind::Boolean | JitKind::String, JitKind::Dynamic)
            | (JitKind::Dynamic, JitKind::Array | JitKind::Dictionary)
            | (JitKind::Array | JitKind::Dictionary, JitKind::Dynamic)
            | (JitKind::Array | JitKind::Dictionary, JitKind::Number | JitKind::Boolean | JitKind::String)
            | (JitKind::Number | JitKind::Boolean | JitKind::String, JitKind::Array | JitKind::Dictionary)
            | (JitKind::Array, JitKind::Dictionary)
            | (JitKind::Dictionary, JitKind::Array)
    ) {
        Some(JitKind::Dynamic)
    } else {
        None
    }
}

fn array_prefix(expression: &[String]) -> Option<&'static str> {
    expression.iter().rev().find_map(|token| {
        if matches!(token.as_str(), "strarray" | "dkeys") {
            return Some("rs");
        }
        match token.as_str() {
            "untagrn" => return Some("rn"),
            "untagrb" => return Some("rb"),
            "untagrs" => return Some("rs"),
            "untagarrayn" => return Some("rn"),
            "untagarrayb" => return Some("rb"),
            "untagarrays" => return Some("rs"),
            "dnvalues" => return Some("rn"),
            "dbvalues" => return Some("rb"),
            "dsvalues" => return Some("rs"),
            _ => {}
        }
        if token.starts_with("objt")
            || token.starts_with("objoptt")
            || token.starts_with("objnullt")
            || token.starts_with("tupoptt")
            || token.starts_with("tupnullt")
        {
            return Some("rn");
        }
        for (field, prefix) in [("objrn", "rn"), ("objrb", "rb"), ("objrs", "rs")] {
            if token.starts_with(field) {
                return Some(prefix);
            }
        }
        for (field, prefix) in [
            ("objoptrn", "rn"),
            ("objoptrb", "rb"),
            ("objoptrs", "rs"),
            ("objnullrn", "rn"),
            ("objnullrb", "rb"),
            ("objnullrs", "rs"),
            ("tupoptrn", "rn"),
            ("tupoptrb", "rb"),
            ("tupoptrs", "rs"),
            ("tupnullrn", "rn"),
            ("tupnullrb", "rb"),
            ("tupnullrs", "rs"),
        ] {
            if token.starts_with(field) {
                return Some(prefix);
            }
        }
        for (field, prefix) in [
            ("ragetrn", "rn"),
            ("ragetrb", "rb"),
            ("ragetrs", "rs"),
        ] {
            if token == field {
                return Some(prefix);
            }
        }
        ["rn", "rb", "rs"]
            .into_iter()
            .find(|prefix| token.starts_with(prefix))
    })
}

fn dictionary_prefix(expression: &[String]) -> Option<&'static str> {
    expression.iter().rev().find_map(|token| {
        match token.as_str() {
            "untagdn" => return Some("dn"),
            "untagdb" => return Some("db"),
            "untagds" => return Some("ds"),
            _ => {}
        }
        if token.starts_with("objo")
            || token.starts_with("objopto")
            || token.starts_with("objnullo")
            || token.starts_with("tupopto")
            || token.starts_with("tupnullo")
        {
            return Some("dn");
        }
        for (field, prefix) in [("objdn", "dn"), ("objdb", "db"), ("objds", "ds")] {
            if token.starts_with(field) {
                return Some(prefix);
            }
        }
        for (field, prefix) in [
            ("objoptdn", "dn"),
            ("objoptdb", "db"),
            ("objoptds", "ds"),
            ("objnulldn", "dn"),
            ("objnulldb", "db"),
            ("objnullds", "ds"),
            ("tupoptdn", "dn"),
            ("tupoptdb", "db"),
            ("tupoptds", "ds"),
            ("tupnulldn", "dn"),
            ("tupnulldb", "db"),
            ("tupnullds", "ds"),
        ] {
            if token.starts_with(field) {
                return Some(prefix);
            }
        }
        for (field, prefix) in [
            ("rogetdn", "dn"),
            ("rogetdb", "db"),
            ("rogetds", "ds"),
        ] {
            if token == field {
                return Some(prefix);
            }
        }
        ["dn", "db", "ds"]
            .into_iter()
            .find(|prefix| token.starts_with(prefix))
    })
}

fn tag_jit_value(expression: &mut Vec<String>, kind: JitKind) -> Option<()> {
    let token = match kind {
        JitKind::Number => "tagnum",
        JitKind::Boolean => "tagbool",
        JitKind::String => "tagstr",
        JitKind::Array => match array_prefix(expression)? {
            "rn" => "tagrn",
            "rb" => "tagrb",
            "rs" => "tagrs",
            _ => return None,
        },
        JitKind::Dictionary => match dictionary_prefix(expression)? {
            "dn" => "tagdn",
            "db" => "tagdb",
            "ds" => "tagds",
            _ => return None,
        },
        JitKind::Dynamic => return Some(()),
    };
    expression.push(token.into());
    Some(())
}

fn normalize_callable_branches<'a>(
    branches: impl IntoIterator<Item = &'a mut Vec<String>>,
) -> Option<JitKind> {
    let mut branches = branches.into_iter().collect::<Vec<_>>();
    let kinds = branches
        .iter()
        .map(|branch| jit_expression_kind(branch).map(|result| result.0))
        .collect::<Option<Vec<_>>>()?;
    let mut kind = kinds
        .iter()
        .copied()
        .try_fold(*kinds.first()?, merge_jit_kinds)?;
    let distinct_aggregates = match kind {
        JitKind::Array => branches
            .iter()
            .map(|branch| array_prefix(branch))
            .collect::<Option<std::collections::HashSet<_>>>()?
            .len()
            > 1,
        JitKind::Dictionary => branches
            .iter()
            .map(|branch| dictionary_prefix(branch))
            .collect::<Option<std::collections::HashSet<_>>>()?
            .len()
            > 1,
        _ => false,
    };
    if distinct_aggregates {
        kind = JitKind::Dynamic;
    }
    if kind == JitKind::Dynamic {
        for (branch, branch_kind) in branches.iter_mut().zip(kinds) {
            tag_jit_value(branch, branch_kind)?;
        }
    }
    Some(kind)
}

fn jit_operation_may_be_absent(operation: &[String]) -> bool {
    if operation.iter().any(|token| {
        matches!(
            token.as_str(),
            "absentn"
                | "absentb"
                | "absents"
                | "absentdyn"
                | "absenta"
                | "absentd"
                | "keepabsentn"
                | "keepabsentb"
                | "keepabsents"
                | "keepabsenta"
                | "nulln"
                | "nullb"
                | "nulls"
                | "nulla"
                | "nulld"
        )
    }) {
        return true;
    }
    let Some(token) = operation.last().map(String::as_str) else {
        return false;
    };
    if token.starts_with("objopt") {
        return true;
    }
    if token.starts_with("objnull") {
        return true;
    }
    if token.starts_with("tupopt") {
        return true;
    }
    if token.starts_with("tupnull") {
        return true;
    }
    matches!(
        token,
        "at"
            | "codepointat"
            | "rnat"
            | "rbat"
            | "rsat"
            | "rnget"
            | "rbget"
            | "rsget"
            | "raget"
            | "roget"
            | "ragetrn"
            | "ragetrb"
            | "ragetrs"
            | "rogetdn"
            | "rogetdb"
            | "rogetds"
            | "rnpop"
            | "rspop"
            | "rbpop"
            | "rnshift"
            | "rsshift"
            | "rbshift"
            | "dynarraypop"
            | "dynarrayshift"
            | "dynarrayfindtruthy"
            | "dynarrayfindlasttruthy"
    ) || ["rn", "rb", "rs", "dynarray"].iter().any(|prefix| {
        token.strip_prefix(prefix).is_some_and(|suffix| {
            suffix.starts_with("find")
                && !suffix.starts_with("findindex")
                && !suffix.starts_with("findlastindex")
        })
    })
}

fn primitive_truthy_result(token: &str) -> Option<JitKind> {
    let (element, operation) = token
        .strip_prefix("rn")
        .map(|operation| (JitKind::Number, operation))
        .or_else(|| {
            token
                .strip_prefix("rb")
                .map(|operation| (JitKind::Boolean, operation))
        })
        .or_else(|| {
            token
                .strip_prefix("rs")
                .map(|operation| (JitKind::String, operation))
        })?;
    match operation {
        "sometruthy" | "everytruthy" => Some(JitKind::Boolean),
        "findtruthy" | "findlasttruthy" => Some(element),
        "findindextruthy" | "findlastindextruthy" => Some(JitKind::Number),
        "filtertruthy" => Some(JitKind::Array),
        _ => None,
    }
}

fn primitive_comparison_result(token: &str) -> Option<(JitKind, JitKind)> {
    let (element, operation) = token
        .strip_prefix("rb")
        .map(|operation| (JitKind::Boolean, operation))
        .or_else(|| token.strip_prefix("rs").map(|operation| (JitKind::String, operation)))?;
    let (method, comparison) = [
        "findlastindex", "findlast", "findindex", "filter", "every", "some", "find",
    ]
    .into_iter()
    .find_map(|method| operation.strip_prefix(method).map(|comparison| (method, comparison)))?;
    if !matches!(comparison, "lt" | "lte" | "gt" | "gte" | "eq" | "ne") {
        return None;
    }
    let result = match method {
        "some" | "every" => JitKind::Boolean,
        "find" | "findlast" => element,
        "findindex" | "findlastindex" => JitKind::Number,
        "filter" => JitKind::Array,
        _ => unreachable!(),
    };
    Some((element, result))
}

fn dynamic_array_comparison_result(token: &str) -> Option<JitKind> {
    let operation = token.strip_prefix("dynarray")?;
    let (method, comparison) = [
        "findlastindex",
        "findlast",
        "findindex",
        "filter",
        "every",
        "some",
        "find",
    ]
    .into_iter()
    .find_map(|method| operation.strip_prefix(method).map(|comparison| (method, comparison)))?;
    if !matches!(comparison, "lt" | "lte" | "gt" | "gte" | "eq" | "ne" | "seq" | "sne") {
        return None;
    }
    Some(match method {
        "some" | "every" => JitKind::Boolean,
        "findindex" | "findlastindex" => JitKind::Number,
        "find" | "findlast" | "filter" => JitKind::Dynamic,
        _ => unreachable!(),
    })
}

fn jit_expression_kind(expression: &[String]) -> Option<(JitKind, usize)> {
    let mut stack = Vec::new();
    let mut branches = Vec::new();
    let mut loops = Vec::new();
    let mut guards = Vec::new();
    let mut switches = Vec::new();
    let mut tries = Vec::new();
    let mut catches = Vec::new();
    let mut dynamic_slots = std::collections::HashSet::new();
    let mut returns = Vec::new();
    let mut maximum_depth = 0;
    for token in expression {
        if let Some((index, _)) = jit_dynamic_argument(token) {
            if index >= 15 {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if let Some(index) = token
            .strip_prefix("setl")
            .and_then(|index| index.parse::<usize>().ok())
        {
            let value = stack.pop()?;
            if stack.get(index).copied()? != value {
                return None;
            }
        } else if let Some((receiver, value, index)) = token
            .strip_prefix("rnlset")
            .map(|index| (JitKind::Array, JitKind::Number, index))
            .or_else(|| token.strip_prefix("rblset").map(|index| (JitKind::Array, JitKind::Boolean, index)))
            .or_else(|| token.strip_prefix("rslset").map(|index| (JitKind::Array, JitKind::String, index)))
            .or_else(|| token.strip_prefix("dnlset").map(|index| (JitKind::Dictionary, JitKind::Number, index)))
            .or_else(|| token.strip_prefix("dblset").map(|index| (JitKind::Dictionary, JitKind::Boolean, index)))
            .or_else(|| token.strip_prefix("dslset").map(|index| (JitKind::Dictionary, JitKind::String, index)))
            .and_then(|(receiver, value, index)| index.parse::<usize>().ok().map(|index| (receiver, value, index)))
        {
            if stack.pop()? != value
                || stack.pop()? != if receiver == JitKind::Array { JitKind::Number } else { JitKind::String }
                || stack.get(index).copied()? != receiver
            {
                return None;
            }
            stack.push(value);
        } else if let Some((value, index)) = token
            .strip_prefix("rnlpush")
            .or_else(|| token.strip_prefix("rnlunshift"))
            .map(|index| (JitKind::Number, index))
            .or_else(|| {
                token
                    .strip_prefix("rblpush")
                    .or_else(|| token.strip_prefix("rblunshift"))
                    .map(|index| (JitKind::Boolean, index))
            })
            .or_else(|| {
                token
                    .strip_prefix("rslpush")
                    .or_else(|| token.strip_prefix("rslunshift"))
                    .map(|index| (JitKind::String, index))
            })
            .and_then(|(value, index)| {
                index
                    .parse::<usize>()
                    .ok()
                    .map(|index| (value, index))
            })
        {
            if stack.pop()? != value || stack.get(index).copied()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Number);
        } else if let Some((kind, index)) = token
            .strip_prefix("rnl")
            .map(|index| (JitKind::Array, index))
            .or_else(|| token.strip_prefix("rbl").map(|index| (JitKind::Array, index)))
            .or_else(|| token.strip_prefix("rsl").map(|index| (JitKind::Array, index)))
            .or_else(|| token.strip_prefix("dnl").map(|index| (JitKind::Dictionary, index)))
            .or_else(|| token.strip_prefix("dbl").map(|index| (JitKind::Dictionary, index)))
            .or_else(|| token.strip_prefix("dsl").map(|index| (JitKind::Dictionary, index)))
            .or_else(|| token.strip_prefix("ln").map(|index| (JitKind::Number, index)))
            .or_else(|| token.strip_prefix("lb").map(|index| (JitKind::Boolean, index)))
            .or_else(|| token.strip_prefix("ls").map(|index| (JitKind::String, index)))
            .or_else(|| token.strip_prefix("ld").map(|index| (JitKind::Dynamic, index)))
            .and_then(|(kind, index)| index.parse::<usize>().ok().map(|index| (kind, index)))
        {
            if let Some(stored) = stack.get(index) {
                if *stored != kind && !dynamic_slots.contains(&index) {
                    return None;
                }
            }
            stack.push(kind);
        } else if token == "loop" {
            loops.push(stack.clone());
        } else if token == "while" {
            if stack.pop()? != JitKind::Boolean || stack.as_slice() != loops.last()?.as_slice() {
                return None;
            }
        } else if token == "looptail" {
            if stack.as_slice() != loops.last()?.as_slice() {
                return None;
            }
        } else if matches!(token.as_str(), "break" | "continue") {
            if !stack.starts_with(loops.last()?) {
                return None;
            }
        } else if token == "loopend" {
            if stack.as_slice() != loops.pop()?.as_slice() {
                return None;
            }
        } else if token == "guard" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            guards.push((stack.clone(), false));
        } else if token == "guardelse" {
            let (base, has_alternate) = guards.last_mut()?;
            if *has_alternate || stack.as_slice() != base.as_slice() {
                return None;
            }
            *has_alternate = true;
        } else if token == "guardend" {
            let (base, _) = guards.pop()?;
            if stack != base {
                return None;
            }
        } else if token == "switch" {
            switches.push(stack.clone());
        } else if token == "case" {
            if stack.as_slice() != switches.last()?.as_slice() {
                return None;
            }
        } else if token == "casebody" {
            if stack.pop()? != JitKind::Boolean
                || stack.as_slice() != switches.last()?.as_slice()
            {
                return None;
            }
        } else if token == "default" {
            if stack.as_slice() != switches.last()?.as_slice() {
                return None;
            }
        } else if token == "switchbreak" {
            if !stack.starts_with(switches.last()?) {
                return None;
            }
        } else if token == "switchend" {
            if stack != switches.pop()? {
                return None;
            }
            stack.pop()?;
        } else if token == "trystart" {
            tries.push((stack.clone(), None, false));
        } else if token == "trystarttag" {
            tries.push((stack.clone(), None, true));
        } else if token == "throw" {
            let thrown = stack.pop()?;
            let (base, kind, tagged) = tries.last_mut()?;
            if *tagged
                || !stack.starts_with(base.as_slice())
                || kind.replace(thrown).is_some_and(|kind| kind != thrown)
            {
                return None;
            }
        } else if let Some(tag) = token
            .strip_prefix("throwtag")
            .and_then(|tag| tag.parse::<u8>().ok())
        {
            stack.pop()?;
            let (base, _, tagged) = tries.last()?;
            if !*tagged || tag > 8 || !stack.starts_with(base.as_slice()) {
                return None;
            }
        } else if token == "checkerror" {
            let (_, kind, tagged) = tries.last_mut()?;
            if !*tagged
                && kind
                .replace(JitKind::String)
                .is_some_and(|kind| kind != JitKind::String)
            {
                return None;
            }
        } else if token == "globalget" {
            if stack.pop()? != JitKind::Number {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(token.as_str(), "globalinit" | "globalset") {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Number {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "callableget" {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "callableset" {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token
            .strip_prefix("dynarraymapto")
            .is_some_and(|target| matches!(target, "number" | "boolean" | "string"))
        {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Array);
        } else if token == "dynarraymapidentity" {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if token
            .strip_prefix("dynarraymapjit")
            .is_some_and(|target| matches!(target, "n" | "b" | "s" | "nc" | "bc" | "sc"))
        {
            let captured = token.ends_with('c');
            if captured && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Array);
        } else if token.strip_prefix("dynarray").is_some_and(|suffix| {
            [
                "somejit",
                "everyjit",
                "findjit",
                "findindexjit",
                "findlastjit",
                "findlastindexjit",
                "filterjit",
            ]
            .iter()
            .any(|operation| suffix == *operation || suffix == format!("{operation}c"))
        }) {
            let captured = token.ends_with('c');
            if captured && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            let method = token
                .strip_prefix("dynarray")?
                .trim_end_matches('c')
                .trim_end_matches("jit");
            stack.push(if matches!(method, "filter" | "find" | "findlast") {
                JitKind::Dynamic
            } else if matches!(method, "some" | "every") {
                JitKind::Boolean
            } else {
                JitKind::Number
            });
        } else if matches!(
            token.as_str(),
            "throwoutn"
                | "throwoutb"
                | "throwouts"
                | "throwoutrn"
                | "throwoutrb"
                | "throwoutrs"
                | "throwoutd"
        ) {
            let expected = match token.as_str() {
                "throwoutn" => JitKind::Number,
                "throwoutb" => JitKind::Boolean,
                "throwouts" => JitKind::String,
                "throwoutrn" | "throwoutrb" | "throwoutrs" => JitKind::Array,
                "throwoutd" => JitKind::Dictionary,
                _ => unreachable!(),
            };
            if stack.pop()? != expected || !stack.starts_with(loops.last()?) {
                return None;
            }
        } else if token == "catch" {
            let (base, kind, tagged) = tries.pop()?;
            if stack != base {
                return None;
            }
            catches.push(base);
            if tagged {
                stack.push(JitKind::Number);
                dynamic_slots.insert(stack.len());
                stack.push(JitKind::Number);
            } else {
                stack.push(kind?);
            }
        } else if token == "tryend" {
            if stack != catches.pop()? {
                return None;
            }
        } else if token == "return" {
            let result = stack.pop()?;
            if !stack.starts_with(loops.last()?) {
                return None;
            }
            returns.push(result);
        } else if token == "dynadd" {
            if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if matches!(
            token.as_str(),
            "dynlt" | "dynlte" | "dyngt" | "dyngte" | "dyneq" | "dynne" | "dynseq" | "dynsne"
        ) {
            if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(
            token.as_str(),
            "+"
                | "-"
                | "*"
                | "/"
                | "%"
                | "min"
                | "max"
                | "pow"
                | "atan2"
                | "hypot"
                | "imul"
                | "band"
                | "bor"
                | "bxor"
                | "shl"
                | "shr"
                | "ushr"
        ) {
            if !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean)
                || !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean)
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if let Some((result, arity)) = token.strip_prefix("recur").and_then(|encoded| {
            let result = match encoded.as_bytes().first()? {
                b'n' => JitKind::Number,
                b'b' => JitKind::Boolean,
                b's' => JitKind::String,
                _ => return None,
            };
            let arity = encoded.get(1..)?.parse::<usize>().ok()?;
            (1..=8).contains(&arity).then_some((result, arity))
        }) {
            for _ in 0..arity {
                stack.pop()?;
            }
            stack.push(result);
        } else if matches!(
            token.as_str(),
            "<" | "<=" | ">" | ">=" | "==" | "!=" | "numsame"
        ) {
            if !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean)
                || !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean)
            {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if token == "strsame" {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if token == "refsame" {
            if stack.pop()? != JitKind::Array || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(token.as_str(), "&&" | "||") {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            let result = stack.pop()?;
            branches.push((stack.len(), Some(result), false));
        } else if token == "if" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            branches.push((stack.len(), None, true));
        } else if token == "ifpresent" {
            let result = *stack.last()?;
            branches.push((stack.len() - 1, Some(result), true));
        } else if token == "else" {
            let (base, expected, awaits_alternate) = branches.last_mut()?;
            if !*awaits_alternate || stack.len() != *base + 1 {
                return None;
            }
            *expected = stack.pop();
            *awaits_alternate = false;
        } else if token == "end" {
            let (base, expected, awaits_alternate) = branches.pop()?;
            let expected = expected?;
            if awaits_alternate || stack.len() != base + 1 {
                return None;
            }
            let alternate = stack.pop()?;
            stack.push(merge_jit_kinds(expected, alternate)?);
        } else if token == "?" {
            let alternative = stack.pop()?;
            let consequent = stack.pop()?;
            if !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean | JitKind::String)
            {
                return None;
            }
            stack.push(merge_jit_kinds(consequent, alternative)?);
        } else if matches!(token.as_str(), "strictfalse" | "stricttrue") {
            stack.pop()?;
            stack.pop()?;
            stack.push(JitKind::Boolean);
        } else if token == "strcmp" {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "rnpop"
                | "rspop"
                | "rbpop"
                | "rnshift"
                | "rsshift"
                | "rbshift"
                | "dynarraypop"
                | "dynarrayshift"
        ) {
            let dynamic = token.starts_with("dynarray");
            if stack.pop()?
                != if dynamic {
                    JitKind::Dynamic
                } else {
                    JitKind::Array
                }
            {
                return None;
            }
            stack.push(if dynamic {
                JitKind::Dynamic
            } else {
                match &token[..2] {
                    "rn" => JitKind::Number,
                    "rs" => JitKind::String,
                    "rb" => JitKind::Boolean,
                    _ => return None,
                }
            });
        } else if matches!(token.as_str(), "dynarraypush" | "dynarrayunshift") {
            if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(token.as_str(), "startswith" | "endswith" | "includes") {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(token.as_str(), "indexof" | "lastindexof") {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "concat" {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(
            token.as_str(),
            "numstr" | "fromcharcode" | "fromcodepoint"
        ) {
            if stack.pop()? != JitKind::Number {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "boolstr" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(token.as_str(), "strnum" | "parsefloat") {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "dynstr" {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "dynnum" {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "parseint" {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "tofixed" | "toprecision" | "toradix" | "toexponential"
        ) {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Number {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "toexponential0" {
            (stack.pop()? == JitKind::Number).then_some(())?;
            stack.push(JitKind::String);
        } else if token == "strbool" {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if token == "dynbool" {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(
            token.as_str(),
            "typeofnumber"
                | "typeofboolean"
                | "typeofstring"
                | "typeofobject"
                | "typeofdynamic"
        ) {
            stack.pop()?;
            stack.push(JitKind::String);
        } else if matches!(
            token.as_str(),
            "notnum" | "notbool" | "notstr" | "notarray" | "notobject"
        ) {
        } else if token == "asbool" {
            if !matches!(*stack.last()?, JitKind::Number | JitKind::Boolean) {
                return None;
            }
            *stack.last_mut()? = JitKind::Boolean;
        } else if token == "boolnot" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(token.as_str(), "isnan" | "isfinite" | "isinteger" | "issafeinteger") {
            if stack.pop()? != JitKind::Number {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(
            token.as_str(),
            "tolowercase" | "touppercase" | "towellformed" | "trim" | "trimstart" | "trimend"
        ) {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "iswellformed" {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if matches!(token.as_str(), "repeat" | "slice" | "substring") {
            if stack.pop()? == JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "normalize" {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::String);
        } else if token == "split" {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(JitKind::Array);
        } else if matches!(
            token.as_str(),
            "dnfromentries" | "dbfromentries" | "dsfromentries"
        ) {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Dictionary);
        } else if matches!(
            token.as_str(),
            "rnfill" | "rsfill" | "rbfill" | "dynarrayfill"
        ) {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Number {
                return None;
            }
            if token == "dynarrayfill" {
                if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                    return None;
                }
                stack.push(JitKind::Dynamic);
            } else {
                let value = stack.pop()?;
                if stack.pop()? != JitKind::Array
                    || value
                        != match &token[..2] {
                            "rn" => JitKind::Number,
                            "rs" => JitKind::String,
                            "rb" => JitKind::Boolean,
                            _ => return None,
                        }
                {
                    return None;
                }
                stack.push(JitKind::Array);
            }
        } else if matches!(token.as_str(), "arraycopywithin" | "dynarraycopywithin") {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()?
                    != if token == "dynarraycopywithin" {
                        JitKind::Dynamic
                    } else {
                        JitKind::Array
                    }
            {
                return None;
            }
            stack.push(if token == "dynarraycopywithin" {
                JitKind::Dynamic
            } else {
                JitKind::Array
            });
        } else if matches!(
            token.as_str(),
            "arraysplice" | "arraytospliced" | "dynarraysplice" | "dynarraytospliced"
        ) {
            let expected = if token.starts_with("dynarray") {
                JitKind::Dynamic
            } else {
                JitKind::Array
            };
            if stack.pop()? != expected
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != expected
            {
                return None;
            }
            stack.push(expected);
        } else if matches!(
            token.as_str(),
            "rnpush" | "rspush" | "rbpush" | "rnunshift" | "rsunshift" | "rbunshift"
        ) {
            let value = stack.pop()?;
            if stack.pop()? != JitKind::Array
                || value
                    != match &token[..2] {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        _ => return None,
                    }
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "rnset" | "rsset" | "rbset" | "dynarrayset"
        ) {
            let value = stack.pop()?;
            if stack.pop()? != JitKind::Number
                || stack.pop()?
                    != if token == "dynarrayset" {
                        JitKind::Dynamic
                    } else {
                        JitKind::Array
                    }
                || value
                    != if token == "dynarrayset" {
                        JitKind::Dynamic
                    } else {
                        match &token[..2] {
                            "rn" => JitKind::Number,
                            "rs" => JitKind::String,
                            "rb" => JitKind::Boolean,
                            _ => return None,
                        }
                    }
            {
                return None;
            }
            stack.push(value);
        } else if token == "rnpostset" {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Array
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "drop" {
            stack.pop()?;
        } else if token == "nip" {
            let value = stack.pop()?;
            stack.pop()?;
            stack.push(value);
        } else if token == "dup" {
            stack.push(*stack.last()?);
        } else if token == "dup2" {
            let length = stack.len();
            if length < 2 {
                return None;
            }
            stack.push(stack[length - 2]);
            stack.push(stack[length - 1]);
        } else if matches!(
            token.as_str(),
            "rnwith" | "rswith" | "rbwith" | "dynarraywith"
        ) {
            let value = stack.pop()?;
            let index = stack.pop()?;
            let array = stack.pop()?;
            let expected = match token.as_str() {
                "rnwith" => (JitKind::Array, JitKind::Number),
                "rswith" => (JitKind::Array, JitKind::String),
                "rbwith" => (JitKind::Array, JitKind::Boolean),
                "dynarraywith" => (JitKind::Dynamic, JitKind::Dynamic),
                _ => return None,
            };
            if index != JitKind::Number || array != expected.0 || value != expected.1 {
                return None;
            }
            stack.push(expected.0);
        } else if matches!(
            token.as_str(),
            "charat" | "charcodeat" | "at" | "codepointat"
        ) {
            if stack.pop()? == JitKind::String || stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(if matches!(token.as_str(), "charat" | "at") {
                JitKind::String
            } else {
                JitKind::Number
            });
        } else if matches!(token.as_str(), "slice2" | "substring2") {
            if stack.pop()? == JitKind::String
                || stack.pop()? == JitKind::String
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(token.as_str(), "padstart" | "padend") {
            if stack.pop()? != JitKind::String
                || stack.pop()? == JitKind::String
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(token.as_str(), "replace" | "replaceall") {
            if stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(
            token.as_str(),
            "startswith2" | "endswith2" | "includes2" | "indexof2" | "lastindexof2"
        ) {
            if stack.pop()? == JitKind::String
                || stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::String
            {
                return None;
            }
            stack.push(if matches!(token.as_str(), "indexof2" | "lastindexof2") {
                JitKind::Number
            } else {
                JitKind::Boolean
            });
        } else if token == "strlen" {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "arraylen" | "rnmin" | "rnmax" | "rnhypot"
        ) {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "dynarraysometruthy"
                | "dynarrayeverytruthy"
                | "dynarrayfindtruthy"
                | "dynarrayfindindextruthy"
                | "dynarrayfindlasttruthy"
                | "dynarrayfindlastindextruthy"
                | "dynarrayfiltertruthy"
        ) {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(match token.as_str() {
                "dynarraysometruthy" | "dynarrayeverytruthy" => JitKind::Boolean,
                "dynarrayfindindextruthy" | "dynarrayfindlastindextruthy" => JitKind::Number,
                _ => JitKind::Dynamic,
            });
        } else if let Some(result) = dynamic_array_comparison_result(token) {
            if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(result);
        } else if let Some(result) = primitive_truthy_result(token) {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(result);
        } else if let Some((element, result)) = primitive_comparison_result(token) {
            if stack.pop()? != element || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(result);
        } else if matches!(
            token.as_str(),
            "rsmapidentity"
                | "rsmaptolowercase"
                | "rsmaptouppercase"
                | "rsmaptrim"
                | "rsmaptrimstart"
                | "rsmaptrimend"
                | "rsmaplength"
                | "rbmapidentity"
                | "rbmapnot"
        ) || token
            .strip_prefix("rnmapto")
            .or_else(|| token.strip_prefix("rbmapto"))
            .or_else(|| token.strip_prefix("rsmapto"))
            .is_some_and(|target| matches!(target, "number" | "boolean" | "string"))
        {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if token
            .strip_prefix("rnreduce")
            .is_some_and(|operation| {
                let operation = operation.strip_prefix("right").unwrap_or(operation);
                matches!(
                    operation.strip_suffix('0').unwrap_or(operation),
                    "add" | "sub" | "mul" | "div" | "rem" | "pow" | "min" | "max"
                )
            })
        {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "dynarrayreducejit"
                | "dynarrayreducejit0"
                | "dynarrayreducerightjit"
                | "dynarrayreducerightjit0"
                | "dynarrayreducejitc"
                | "dynarrayreducejitc0"
                | "dynarrayreducerightjitc"
                | "dynarrayreducerightjitc0"
        ) {
            if token.contains("jitc") && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Dynamic
            {
                return None;
            }
            stack.push(if token.ends_with('0') {
                JitKind::Dynamic
            } else {
                JitKind::Number
            });
        } else if matches!(
            token.as_str(),
            "rnreducejit"
                | "rnreducejit0"
                | "rnreducerightjit"
                | "rnreducerightjit0"
                | "rnreducejitc"
                | "rnreducejitc0"
                | "rnreducerightjitc"
                | "rnreducerightjitc0"
        ) {
            if token.contains("jitc") && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Array
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token
            .strip_prefix("rnsome")
            .or_else(|| token.strip_prefix("rnevery"))
            .is_some_and(|operation| matches!(operation, "lt" | "lte" | "gt" | "gte" | "eq" | "ne"))
        {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if token
            .strip_prefix("rnfindlastindex")
            .or_else(|| token.strip_prefix("rnfindlast"))
            .or_else(|| token.strip_prefix("rnfindindex"))
            .or_else(|| token.strip_prefix("rnfind"))
            .is_some_and(|operation| matches!(operation, "lt" | "lte" | "gt" | "gte" | "eq" | "ne"))
        {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token
            .strip_prefix("rnmapbranch")
            .and_then(|encoded| u16::from_str_radix(encoded, 16).ok())
            .is_some_and(|encoded| {
                encoded & 7 <= 5
                    && (encoded >> 4) & 15 <= 13
                    && (encoded >> 8) & 15 <= 13
            })
            || token
                .strip_prefix("rnmapselect")
                .is_some_and(|encoded| {
                    encoded
                        .strip_suffix(['0', '1', '2', '3'])
                        .is_some_and(|operation| {
                            matches!(operation, "lt" | "lte" | "gt" | "gte" | "eq" | "ne")
                        })
                })
        {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if token
            .strip_prefix("rnmapindex")
            .is_some_and(|operation| {
                matches!(operation, "add" | "sub" | "mul" | "div" | "rem" | "pow")
                    || matches!(
                        operation.strip_prefix('r'),
                        Some("add" | "sub" | "mul" | "div" | "rem" | "pow")
                    )
            })
        {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if matches!(token.as_str(), "rnmapjit" | "rnmapjitc")
            || ["rn", "rb", "rs"].iter().any(|prefix| {
                token.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(suffix, "mapjitn" | "mapjitb" | "mapjits" | "mapjitnc" | "mapjitbc" | "mapjitsc")
                })
            })
        {
            if token.ends_with('c') && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if ["rn", "rb", "rs"].iter().any(|prefix| {
            token.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "somejit"
                        | "everyjit"
                        | "findjit"
                        | "findindexjit"
                        | "findlastjit"
                        | "findlastindexjit"
                        | "filterjit"
                        | "somejitc"
                        | "everyjitc"
                        | "findjitc"
                        | "findindexjitc"
                        | "findlastjitc"
                        | "findlastindexjitc"
                        | "filterjitc"
                )
            })
        }) {
            if token.ends_with("jitc") && stack.pop()? != JitKind::Array {
                return None;
            }
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::Array {
                return None;
            }
            let suffix = &token[2..];
            stack.push(if matches!(suffix, "filterjit" | "filterjitc") {
                JitKind::Array
            } else if matches!(suffix, "somejit" | "everyjit" | "somejitc" | "everyjitc") {
                JitKind::Boolean
            } else if matches!(suffix, "findjit" | "findlastjit" | "findjitc" | "findlastjitc") {
                match &token[..2] {
                    "rn" => JitKind::Number,
                    "rb" => JitKind::Boolean,
                    "rs" => JitKind::String,
                    _ => return None,
                }
            } else {
                JitKind::Number
            });
        } else if token
            .strip_prefix("rnfilter")
            .is_some_and(|operation| matches!(operation, "lt" | "lte" | "gt" | "gte" | "eq" | "ne"))
            || token.strip_prefix("rnmap").is_some_and(|operation| {
                matches!(operation, "add" | "sub" | "mul" | "div" | "rem" | "pow")
                    || matches!(
                        operation.strip_prefix('r'),
                        Some("add" | "sub" | "mul" | "div" | "rem" | "pow")
                    )
            })
        {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if matches!(token.as_str(), "arrayvalue" | "arrayhandle")
            || matches!(token.as_str(), "rnmapneg" | "rnmapabs")
            || token
                .strip_prefix("rnmap")
                .is_some_and(|operation| operation != "abs" && is_unary_math_method(operation))
        {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if token == "arrayempty" {
            stack.push(JitKind::Array);
        } else if token == "strarray" {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Array);
        } else if matches!(token.as_str(), "arrayslice" | "dynarrayslice") {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()?
                    != if token == "dynarrayslice" {
                        JitKind::Dynamic
                    } else {
                        JitKind::Array
                    }
            {
                return None;
            }
            stack.push(if token == "dynarrayslice" {
                JitKind::Dynamic
            } else {
                JitKind::Array
            });
        } else if matches!(token.as_str(), "arrayconcat" | "dynarrayconcat") {
            let expected = if token == "dynarrayconcat" {
                JitKind::Dynamic
            } else {
                JitKind::Array
            };
            if stack.pop()? != expected || stack.pop()? != expected {
                return None;
            }
            stack.push(expected);
        } else if token == "captureappend" {
            stack.pop()?;
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Array);
        } else if matches!(
            token.as_str(),
            "rnappend" | "rsappend" | "rbappend" | "dynarrayappend"
        ) {
            let value = stack.pop()?;
            if token == "dynarrayappend" {
                if value != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                    return None;
                }
                stack.push(JitKind::Dynamic);
            } else if stack.pop()? != JitKind::Array
                || value
                    != match &token[..2] {
                        "rn" => JitKind::Number,
                        "rs" => JitKind::String,
                        "rb" => JitKind::Boolean,
                        _ => return None,
                    }
            {
                return None;
            } else {
                stack.push(JitKind::Array);
            }
        } else if matches!(
            token.as_str(),
            "arrayreversed"
                | "arrayreverse"
                | "dynarrayreversed"
                | "dynarrayreverse"
                | "dynarraysorted"
                | "dynarraysort"
                | "rnsorted"
                | "rssorted"
                | "rbsorted"
                | "rnsort"
                | "rssort"
                | "rbsort"
                | "rnsortedasc"
                | "rnsorteddesc"
                | "rnsortasc"
                | "rnsortdesc"
                | "rssorteddesc"
                | "rssortdesc"
        ) {
            let expected = if token.starts_with("dynarray") {
                JitKind::Dynamic
            } else {
                JitKind::Array
            };
            if stack.pop()? != expected {
                return None;
            }
            stack.push(expected);
        } else if matches!(token.as_str(), "isarray" | "isnotarray" | "dynisarray") {
            stack.pop()?;
            stack.push(JitKind::Boolean);
        } else if token == "dlen" {
            if stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(JitKind::Number);
        } else if token == "dkeyat" {
            if stack.pop()? != JitKind::Number || stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(token.as_str(), "dnget" | "dbget" | "dsget") {
            if stack.pop()? != JitKind::String || stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(match token.as_str() {
                "dnget" => JitKind::Number,
                "dbget" => JitKind::Boolean,
                "dsget" => JitKind::String,
                _ => unreachable!(),
            });
        } else if matches!(token.as_str(), "dnempty" | "dbempty" | "dsempty") {
            stack.push(JitKind::Dictionary);
        } else if let Some(value) = token
            .strip_prefix("dnput")
            .map(|_| JitKind::Number)
            .or_else(|| token.strip_prefix("dbput").map(|_| JitKind::Boolean))
            .or_else(|| token.strip_prefix("dsput").map(|_| JitKind::String))
        {
            if stack.pop()? != value || stack.last().copied()? != JitKind::Dictionary {
                return None;
            }
        } else if matches!(token.as_str(), "dnappend" | "dbappend" | "dsappend") {
            let value = stack.pop()?;
            if stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::Dictionary
                || value
                    != match token.as_str() {
                        "dnappend" => JitKind::Number,
                        "dbappend" => JitKind::Boolean,
                        "dsappend" => JitKind::String,
                        _ => unreachable!(),
                    }
            {
                return None;
            }
            stack.push(JitKind::Dictionary);
        } else if matches!(token.as_str(), "dnset" | "dbset" | "dsset") {
            let value = stack.pop()?;
            if stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::Dictionary
                || value
                    != match token.as_str() {
                        "dnset" => JitKind::Number,
                        "dbset" => JitKind::Boolean,
                        "dsset" => JitKind::String,
                        _ => unreachable!(),
                    }
            {
                return None;
            }
            stack.push(value);
        } else if token == "dnpostset" {
            if stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::Number
                || stack.pop()? != JitKind::String
                || stack.pop()? != JitKind::Dictionary
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(token.as_str(), "ddelete" | "dhasown" | "din") {
            let expected = if token == "din" {
                (JitKind::Dictionary, JitKind::String)
            } else {
                (JitKind::String, JitKind::Dictionary)
            };
            if stack.pop()? != expected.0 || stack.pop()? != expected.1 {
                return None;
            }
            stack.push(JitKind::Boolean);
        } else if token == "dassign" {
            if stack.pop()? != JitKind::Dictionary || stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(JitKind::Dictionary);
        } else if matches!(
            token.as_str(),
            "dkeys"
                | "dnvalues"
                | "dbvalues"
                | "dsvalues"
                | "dnentries"
                | "dbentries"
                | "dsentries"
        ) {
            if stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(JitKind::Array);
        } else if let Some((kind, index)) = [
            ("tupnulln", JitKind::Number),
            ("tupnullb", JitKind::Boolean),
            ("tupnulls", JitKind::String),
            ("tupnullo", JitKind::Dictionary),
            ("tupnullt", JitKind::Array),
            ("tupnullrn", JitKind::Array),
            ("tupnullrb", JitKind::Array),
            ("tupnullrs", JitKind::Array),
            ("tupnulldn", JitKind::Dictionary),
            ("tupnulldb", JitKind::Dictionary),
            ("tupnullds", JitKind::Dictionary),
            ("tupoptn", JitKind::Number),
            ("tupoptb", JitKind::Boolean),
            ("tupopts", JitKind::String),
            ("tupopto", JitKind::Dictionary),
            ("tupoptt", JitKind::Array),
            ("tupoptrn", JitKind::Array),
            ("tupoptrb", JitKind::Array),
            ("tupoptrs", JitKind::Array),
            ("tupoptdn", JitKind::Dictionary),
            ("tupoptdb", JitKind::Dictionary),
            ("tupoptds", JitKind::Dictionary),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| token.strip_prefix(prefix).map(|index| (kind, index)))
        {
            if stack.pop()? != JitKind::Array || index.parse::<u16>().is_err() {
                return None;
            }
            stack.push(kind);
        } else if matches!(
            token.as_str(),
            "rnat"
                | "rbat"
                | "rsat"
                | "dynarrayat"
                | "rnget"
                | "rbget"
                | "rsget"
                | "raget"
                | "roget"
                | "ragetrn"
                | "ragetrb"
                | "ragetrs"
                | "rogetdn"
                | "rogetdb"
                | "rogetds"
        ) {
            if stack.pop()? != JitKind::Number
                || stack.pop()?
                    != if token == "dynarrayat" {
                        JitKind::Dynamic
                    } else {
                        JitKind::Array
                    }
            {
                return None;
            }
            stack.push(match token.as_str() {
                "rnat" | "rnget" => JitKind::Number,
                "rbat" | "rbget" => JitKind::Boolean,
                "rsat" | "rsget" => JitKind::String,
                "raget" => JitKind::Array,
                "roget" => JitKind::Dictionary,
                "ragetrn" | "ragetrb" | "ragetrs" => JitKind::Array,
                "rogetdn" | "rogetdb" | "rogetds" => JitKind::Dictionary,
                "dynarrayat" => JitKind::Dynamic,
                _ => unreachable!(),
            });
        } else if matches!(
            token.as_str(),
            "rnjoin" | "rbjoin" | "rsjoin" | "dynarrayjoin"
        ) {
            if stack.pop()? != JitKind::String
                || stack.pop()?
                    != if token == "dynarrayjoin" {
                        JitKind::Dynamic
                    } else {
                        JitKind::Array
                    }
            {
                return None;
            }
            stack.push(JitKind::String);
        } else if matches!(
            token.as_str(),
            "rnincludes"
                | "rbincludes"
                | "rsincludes"
                | "dynarrayincludes"
                | "rnindexof"
                | "rbindexof"
                | "rsindexof"
                | "dynarrayindexof"
                | "rnlastindexof"
                | "rblastindexof"
                | "rslastindexof"
                | "dynarraylastindexof"
        ) {
            if stack.pop()? != JitKind::Number {
                return None;
            }
            if token.starts_with("dynarray") {
                if stack.pop()? != JitKind::Dynamic || stack.pop()? != JitKind::Dynamic {
                    return None;
                }
            } else {
                let needle = stack.pop()?;
                if stack.pop()? != JitKind::Array
                    || needle
                        != match &token[..2] {
                            "rn" => JitKind::Number,
                            "rb" => JitKind::Boolean,
                            "rs" => JitKind::String,
                            _ => return None,
                        }
                {
                    return None;
                }
            }
            stack.push(if token.ends_with("includes") {
                JitKind::Boolean
            } else {
                JitKind::Number
            });
        } else if matches!(
            token.as_str(),
            "acos"
                | "acosh"
                | "asin"
                | "asinh"
                | "atan"
                | "atanh"
                | "cbrt"
                | "ceil"
                | "clz32"
                | "cos"
                | "cosh"
                | "exp"
                | "expm1"
                | "floor"
                | "fround"
                | "log"
                | "log1p"
                | "log2"
                | "log10"
                | "round"
                | "sign"
                | "sin"
                | "sinh"
                | "tan"
                | "tanh"
                | "trunc"
                | "bnot"
                | "neg"
                | "abs"
                | "sqrt"
        ) {
            if !matches!(*stack.last()?, JitKind::Number | JitKind::Boolean) {
                return None;
            }
            *stack.last_mut()? = JitKind::Number;
        } else if token == "tagnum" {
            if !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean) {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if token == "tagstr" {
            if stack.pop()? != JitKind::String {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if token == "tagbool" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if matches!(token.as_str(), "tagrn" | "tagrb" | "tagrs") {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if matches!(
            token.as_str(),
            "tagdn" | "tagdb" | "tagds" | "tagobject"
        ) {
            if stack.pop()? != JitKind::Dictionary {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if token == "tagtuple" {
            if stack.pop()? != JitKind::Array {
                return None;
            }
            stack.push(JitKind::Dynamic);
        } else if let Some(size) = token.strip_prefix("objnew") {
            size.parse::<u16>().ok().filter(|size| *size > 0)?;
            stack.push(JitKind::Dictionary);
        } else if let Some(length) = token.strip_prefix("tupneww") {
            length.parse::<u16>().ok()?;
            stack.push(JitKind::Array);
        } else if let Some(length) = token.strip_prefix("tupnew") {
            length.parse::<u16>().ok()?;
            stack.push(JitKind::Array);
        } else if let Some((tagged, encoded)) = token
            .strip_prefix("tupsetnull")
            .map(|encoded| (true, encoded))
            .or_else(|| {
                token
                    .strip_prefix("tupsetopt")
                    .map(|encoded| (true, encoded))
            })
            .or_else(|| token.strip_prefix("tupsetw").map(|encoded| (false, encoded)))
        {
            let (kind, index) = encoded.split_at(1);
            let value = stack.pop()?;
            if stack.pop()? != JitKind::Array
                || index.parse::<u16>().is_err()
                || !match kind {
                    "n" | "b" => matches!(value, JitKind::Number | JitKind::Boolean),
                    "s" => value == JitKind::String,
                    "p" => value == JitKind::Array,
                    "o" => value == JitKind::Dictionary,
                    "u" => matches!(value, JitKind::Number | JitKind::Boolean),
                    _ => return None,
                }
                || tagged && !matches!(kind, "n" | "b" | "s" | "p" | "o")
            {
                return None;
            }
            stack.push(JitKind::Array);
        } else if let Some(encoded) = token.strip_prefix("tupset") {
            let (kind, index) = encoded.split_at(1);
            let value = stack.pop()?;
            if stack.pop()? != JitKind::Array
                || index.parse::<u16>().is_err()
                || !match kind {
                    "n" => matches!(value, JitKind::Number | JitKind::Boolean),
                    "b" => matches!(value, JitKind::Number | JitKind::Boolean),
                    "s" => value == JitKind::String,
                    "p" => value == JitKind::Array,
                    "o" => value == JitKind::Dictionary,
                    _ => return None,
                }
            {
                return None;
            }
            stack.push(JitKind::Array);
        } else if let Some(encoded) = token.strip_prefix("objca") {
            if !matches!(stack.pop()?, JitKind::Number | JitKind::Boolean)
                || stack.pop()? != JitKind::Dictionary
                || !matches!(encoded.as_bytes(), [b'o' | b'l' | b'n', b'a' | b's' | b'm' | b'd' | b'r' | b'l' | b'h' | b'u' | b'o' | b'x' | b'b' | b'p', rest @ ..] if !rest.is_empty() && rest.iter().all(u8::is_ascii_digit))
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if let Some(encoded) = token.strip_prefix("objup") {
            if stack.pop()? != JitKind::Dictionary
                || !matches!(encoded.as_bytes(), [b'o' | b'l' | b'n', b'i' | b'd', b'p' | b'o', rest @ ..] if !rest.is_empty() && rest.iter().all(u8::is_ascii_digit))
            {
                return None;
            }
            stack.push(JitKind::Number);
        } else if let Some(encoded) = token.strip_prefix("objset") {
            let (kind, offset) = encoded.split_at(1);
            let value = stack.pop()?;
            if stack.pop()? != JitKind::Dictionary
                || offset.parse::<u16>().is_err()
                || !match kind {
                    "n" => matches!(value, JitKind::Number | JitKind::Boolean),
                    "b" => matches!(value, JitKind::Number | JitKind::Boolean),
                    "s" => value == JitKind::String,
                    "a" => value == JitKind::Array,
                    "o" => value == JitKind::Dictionary,
                    "u" => matches!(value, JitKind::Number | JitKind::Boolean),
                    _ => return None,
                }
            {
                return None;
            }
            stack.push(JitKind::Dictionary);
        } else if token == "tagkind" {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(JitKind::Number);
        } else if matches!(
            token.as_str(),
            "untagnum"
                | "untagstr"
                | "untagbool"
                | "untagrn"
                | "untagrb"
                | "untagrs"
                | "untagarray"
                | "untagarrayn"
                | "untagarrayb"
                | "untagarrays"
                | "untagdn"
                | "untagdb"
                | "untagds"
                | "untagdictionary"
                | "untagobject"
                | "untagtuple"
        ) {
            if stack.pop()? != JitKind::Dynamic {
                return None;
            }
            stack.push(match token.as_str() {
                "untagnum" => JitKind::Number,
                "untagbool" => JitKind::Boolean,
                "untagstr" => JitKind::String,
                "untagrn"
                | "untagrb"
                | "untagrs"
                | "untagarray"
                | "untagarrayn"
                | "untagarrayb"
                | "untagarrays"
                | "untagtuple" => JitKind::Array,
                _ => JitKind::Dictionary,
            });
        } else if let Some((kind, offset)) = [
            ("objnulln", JitKind::Number),
            ("objnullablen", JitKind::Number),
            ("objnullb", JitKind::Boolean),
            ("objnulls", JitKind::String),
            ("objnullo", JitKind::Dictionary),
            ("objnullt", JitKind::Array),
            ("objnullrn", JitKind::Array),
            ("objnullrb", JitKind::Array),
            ("objnullrs", JitKind::Array),
            ("objnulldn", JitKind::Dictionary),
            ("objnulldb", JitKind::Dictionary),
            ("objnullds", JitKind::Dictionary),
            ("objoptn", JitKind::Number),
            ("objoptb", JitKind::Boolean),
            ("objopts", JitKind::String),
            ("objopto", JitKind::Dictionary),
            ("objoptt", JitKind::Array),
            ("objoptrn", JitKind::Array),
            ("objoptrb", JitKind::Array),
            ("objoptrs", JitKind::Array),
            ("objoptdn", JitKind::Dictionary),
            ("objoptdb", JitKind::Dictionary),
            ("objoptds", JitKind::Dictionary),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| token.strip_prefix(prefix).map(|offset| (kind, offset)))
        {
            if stack.pop()? != JitKind::Dictionary || offset.parse::<u16>().is_err() {
                return None;
            }
            stack.push(kind);
        } else if let Some((kind, offset)) = [
            ("objn", JitKind::Number),
            ("objb", JitKind::Boolean),
            ("objs", JitKind::String),
            ("objrn", JitKind::Array),
            ("objrb", JitKind::Array),
            ("objrs", JitKind::Array),
            ("objo", JitKind::Dictionary),
            ("objt", JitKind::Array),
            ("objdn", JitKind::Dictionary),
            ("objdb", JitKind::Dictionary),
            ("objds", JitKind::Dictionary),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| token.strip_prefix(prefix).map(|offset| (kind, offset)))
        {
            if stack.pop()? != JitKind::Dictionary || offset.parse::<u16>().is_err() {
                return None;
            }
            stack.push(kind);
        } else if matches!(
            token.as_str(),
            "missingcalln"
                | "missingcallb"
                | "missingcalls"
                | "missingcalldyn"
                | "missingcalla"
                | "missingcalld"
        ) {
            stack.push(match token.as_str() {
                "missingcalln" => JitKind::Number,
                "missingcallb" => JitKind::Boolean,
                "missingcalls" => JitKind::String,
                "missingcalldyn" => JitKind::Dynamic,
                "missingcalla" => JitKind::Array,
                "missingcalld" => JitKind::Dictionary,
                _ => unreachable!(),
            });
            maximum_depth = maximum_depth.max(stack.len());
        } else if matches!(
            token.as_str(),
            "absentn"
                | "absentb"
                | "absents"
                | "absentdyn"
                | "absenta"
                | "absentd"
                | "keepabsentn"
                | "keepabsentb"
                | "keepabsents"
                | "keepabsenta"
                | "nulln"
                | "nullb"
                | "nulls"
                | "nulla"
                | "nulld"
        ) {
            stack.push(match token.as_str() {
                "absentn" => JitKind::Number,
                "keepabsentn" => JitKind::Number,
                "nulln" => JitKind::Number,
                "absentb" => JitKind::Boolean,
                "keepabsentb" => JitKind::Boolean,
                "nullb" => JitKind::Boolean,
                "absents" => JitKind::String,
                "keepabsents" => JitKind::String,
                "nulls" => JitKind::String,
                "keepabsenta" => JitKind::Array,
                "nulla" => JitKind::Array,
                "nulld" => JitKind::Dictionary,
                "absentdyn" => JitKind::Dynamic,
                "absenta" => JitKind::Array,
                "absentd" => JitKind::Dictionary,
                _ => unreachable!(),
            });
            maximum_depth = maximum_depth.max(stack.len());
        } else {
            stack.push(if token.starts_with('s') || token.starts_with('t') {
                JitKind::String
            } else if token.starts_with('b') {
                JitKind::Boolean
            } else if token.starts_with("rn")
                || token.starts_with("rb")
                || token.starts_with("rs")
                || token.starts_with("en")
                || token.starts_with("eb")
                || token.starts_with("es")
            {
                JitKind::Array
            } else if token.starts_with("dn")
                || token.starts_with("db")
                || token.starts_with("ds")
            {
                JitKind::Dictionary
            } else {
                JitKind::Number
            });
            maximum_depth = maximum_depth.max(stack.len());
        }
    }
    let [kind] = stack.as_slice() else {
        return None;
    };
    let kind = returns
        .into_iter()
        .try_fold(*kind, merge_jit_kinds)?;
    (branches.is_empty()
        && loops.is_empty()
        && guards.is_empty()
        && switches.is_empty()
        && tries.is_empty()
        && catches.is_empty())
    .then_some((kind, maximum_depth))
}

fn validated_jit_expression(mut expression: Vec<String>, expected: JitKind) -> Option<String> {
    let (mut kind, maximum_depth) = jit_expression_kind(&expression)?;
    if expected == JitKind::Dynamic && kind != JitKind::Dynamic {
        tag_jit_value(&mut expression, kind)?;
        kind = JitKind::Dynamic;
    }
    let compatible = kind == expected
        || (matches!(kind, JitKind::Number | JitKind::Boolean)
            && matches!(expected, JitKind::Number | JitKind::Boolean));
    if !compatible
        || maximum_depth > 8
        || (expected == JitKind::Dynamic
            && !expression
                .iter()
                .any(|token| {
                    matches!(
                        token.as_str(),
                        "tagnum"
                            | "tagbool"
                            | "tagstr"
                            | "tagrn"
                            | "tagrb"
                            | "tagrs"
                            | "tagdn"
                            | "tagdb"
                            | "tagds"
                            | "tagobject"
                            | "tagtuple"
                    )
                        || jit_dynamic_argument(token).is_some()
                }))
    {
        return None;
    }
    if expected == JitKind::Array {
        if expression
            .iter()
            .any(|token| matches!(token.as_str(), "absenta" | "nulla"))
        {
            expression.extend(
                ["ifpresent", "arrayvalue", "else", "keepabsenta", "end"]
                    .map(str::to_owned),
            );
        } else {
            expression.push("arrayvalue".into());
        }
    }
    Some(format!("expr:{}", expression.join(",")))
}

fn jit_numeric_declaration(
    package: &str,
    function: &thaw_bridge::DtsFunction,
    operation: &JitExport,
) -> (String, String) {
    fn declaration_params(
        function: &thaw_bridge::DtsFunction,
    ) -> Vec<(&str, String, bool)> {
        function
            .params
            .iter()
            .enumerate()
            .map(|(index, (name, ty))| {
                let (ty, optional_type) = match ty {
                    thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload)) => {
                        (payload.as_ref(), true)
                    }
                    thaw_bridge::DtsType::Native(ty) => (ty, false),
                    thaw_bridge::DtsType::Unsupported(_) => unreachable!(),
                };
                (
                    name.as_str(),
                    render_dynamic_type(ty).unwrap(),
                    index >= function.required_params || optional_type,
                )
            })
            .collect()
    }

    fn encoded_symbol(runtime_key: &str) -> String {
        let encoded = runtime_key
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("__thaw_typed_jit_{encoded}")
    }

    fn emit_aggregate_call(
        runtime_name: &str,
        value: &JitExport,
        ty: &thaw_hir::HirType,
        path: &mut Vec<String>,
        direct_params: &str,
        arguments: &str,
        declaration: &mut String,
    ) -> String {
        match value {
            JitExport::Null => return "null".into(),
            JitExport::Undefined => return "undefined".into(),
            JitExport::Conditional(_, _, _) => {}
            _ => {
                if let thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload) = ty
                {
                    return emit_aggregate_call(
                        runtime_name,
                        value,
                        payload,
                        path,
                        direct_params,
                        arguments,
                        declaration,
                    );
                }
            }
        }
        match value {
            JitExport::Value(operation) => {
                let runtime_key = format!("{operation}:{runtime_name}:{}", path.join("."));
                let symbol = encoded_symbol(&runtime_key);
                declaration.push_str(&format!(
                    "declare function {symbol}({direct_params}): {};\n",
                    render_dynamic_type(ty).unwrap()
                ));
                format!("{symbol}({arguments})")
            }
            JitExport::Object(fields) => {
                let thaw_hir::HirType::Object(types) = ty else {
                    unreachable!()
                };
                let values = fields
                    .iter()
                    .map(|(name, value)| {
                        path.push(name.clone());
                        let ty = &types.iter().find(|(field, _)| field == name).unwrap().1;
                        let expression = emit_aggregate_call(
                            runtime_name,
                            value,
                            ty,
                            path,
                            direct_params,
                            arguments,
                            declaration,
                        );
                        path.pop();
                        format!("{}: {expression}", serde_json::to_string(name).unwrap())
                    })
                    .collect::<Vec<_>>();
                format!("{{ {} }}", values.join(", "))
            }
            JitExport::Dictionary(fields) => {
                let thaw_hir::HirType::Dictionary(element) = ty else {
                    unreachable!()
                };
                let values = fields
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        path.push(format!("entry{index}"));
                        let expression = match entry {
                            JitDictionaryEntry::Static(name, value) => format!(
                                "result[{}] = {};",
                                serde_json::to_string(name).unwrap(),
                                emit_aggregate_call(
                                    runtime_name,
                                    value,
                                    element,
                                    path,
                                    direct_params,
                                    arguments,
                                    declaration,
                                )
                            ),
                            JitDictionaryEntry::Computed(key, value) => {
                                path.push("key".into());
                                let key = emit_aggregate_call(
                                    runtime_name,
                                    key,
                                    &thaw_hir::HirType::Str,
                                    path,
                                    direct_params,
                                    arguments,
                                    declaration,
                                );
                                path.pop();
                                path.push("value".into());
                                let value = emit_aggregate_call(
                                    runtime_name,
                                    value,
                                    element,
                                    path,
                                    direct_params,
                                    arguments,
                                    declaration,
                                );
                                path.pop();
                                format!("result[{key}] = {value};")
                            }
                            JitDictionaryEntry::Spread(value) => format!(
                                "Object.assign(result, {});",
                                emit_aggregate_call(
                                    runtime_name,
                                    value,
                                    ty,
                                    path,
                                    direct_params,
                                    arguments,
                                    declaration,
                                )
                            ),
                        };
                        path.pop();
                        expression
                    })
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    "{}".into()
                } else {
                    let ty = render_dynamic_type(ty).unwrap();
                    let helper = format!(
                        "__thaw_jit_dictionary_{}",
                        format!("{runtime_name}:{}", path.join("."))
                            .as_bytes()
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    );
                    declaration.push_str(&format!(
                        "function {helper}({direct_params}): {ty} {{ const result: {ty} = {{}}; {} return result; }}\n",
                        values.join(" ")
                    ));
                    format!("{helper}({arguments})")
                }
            }
            JitExport::Tuple(values) => {
                let thaw_hir::HirType::Tuple(types) = ty else {
                    unreachable!()
                };
                let values = values
                    .iter()
                    .zip(types)
                    .enumerate()
                    .map(|(index, (value, ty))| {
                        path.push(index.to_string());
                        let expression = emit_aggregate_call(
                            runtime_name,
                            value,
                            ty,
                            path,
                            direct_params,
                            arguments,
                            declaration,
                        );
                        path.pop();
                        expression
                    })
                    .collect::<Vec<_>>();
                format!("[{}]", values.join(", "))
            }
            JitExport::Conditional(condition, consequent, alternate) => {
                let runtime_key = format!(
                    "{condition}:{runtime_name}:{}.condition",
                    path.join(".")
                );
                let symbol = encoded_symbol(&runtime_key);
                declaration.push_str(&format!(
                    "declare function {symbol}({direct_params}): boolean;\n"
                ));
                let consequent = emit_aggregate_call(
                    runtime_name,
                    consequent,
                    ty,
                    path,
                    direct_params,
                    arguments,
                    declaration,
                );
                let alternate = emit_aggregate_call(
                    runtime_name,
                    alternate,
                    ty,
                    path,
                    direct_params,
                    arguments,
                    declaration,
                );
                format!("{symbol}({arguments}) ? {consequent} : {alternate}")
            }
            JitExport::WithLocals(_, _) => unreachable!(),
            JitExport::Null | JitExport::Undefined => unreachable!(),
        }
    }

    fn emit_aggregate_return(
        runtime_name: &str,
        value: &JitExport,
        ty: &thaw_hir::HirType,
        path: &mut Vec<String>,
        direct_params: &str,
        arguments: &str,
        declaration: &mut String,
    ) -> String {
        if let JitExport::Conditional(condition, consequent, alternate) = value {
            let runtime_key = format!(
                "{condition}:{runtime_name}:{}.condition",
                path.join(".")
            );
            let symbol = encoded_symbol(&runtime_key);
            declaration.push_str(&format!(
                "declare function {symbol}({direct_params}): boolean;\n"
            ));
            return format!(
                "if ({symbol}({arguments})) {{ {} }} else {{ {} }}",
                emit_aggregate_return(
                    runtime_name,
                    consequent,
                    ty,
                    path,
                    direct_params,
                    arguments,
                    declaration,
                ),
                emit_aggregate_return(
                    runtime_name,
                    alternate,
                    ty,
                    path,
                    direct_params,
                    arguments,
                    declaration,
                )
            );
        }
        format!(
            "return {};",
            emit_aggregate_call(
                runtime_name,
                value,
                ty,
                path,
                direct_params,
                arguments,
                declaration,
            )
        )
    }

    let params = declaration_params(function);
    let direct_params = params
        .iter()
        .map(|(name, ty, optional)| {
            if *optional {
                format!("{name}: ({ty}) | undefined")
            } else {
                format!("{name}: {ty}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let (jit_locals, aggregate) = match operation {
        JitExport::Object(_)
        | JitExport::Null
        | JitExport::Undefined
        | JitExport::Dictionary(_)
        | JitExport::Tuple(_)
        | JitExport::Conditional(_, _, _) => {
            (&[][..], operation)
        }
        JitExport::WithLocals(locals, aggregate) => (locals.as_slice(), aggregate.as_ref()),
        JitExport::Value(_) => (&[][..], operation),
    };
    if !jit_locals.is_empty()
        || matches!(
            aggregate,
            JitExport::Object(_)
            | JitExport::Null
            | JitExport::Undefined
            | JitExport::Dictionary(_)
            | JitExport::Tuple(_)
            | JitExport::Conditional(_, _, _)
        )
    {
        let thaw_bridge::DtsType::Native(return_type) = &function.ret else {
            unreachable!()
        };
        let mut arguments = params
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        let mut declaration = String::new();
        let runtime_name = format!("{package}::{}", function.name);
        let mut leaf_params = direct_params.clone();
        let mut local_statements = String::new();
        for (index, local) in jit_locals.iter().enumerate() {
            let local_name = format!("__thaw_jit_local_{index}");
            let local_type = render_dynamic_type(&local.ty).unwrap();
            let symbol = encoded_symbol(&format!(
                "{}:{runtime_name}:local.{index}",
                local.operation
            ));
            declaration.push_str(&format!(
                "declare function {symbol}({leaf_params}): {local_type};\n"
            ));
            local_statements.push_str(&format!(
                "const {local_name}: {local_type} = {symbol}({arguments}); "
            ));
            if !leaf_params.is_empty() {
                leaf_params.push_str(", ");
                arguments.push_str(", ");
            }
            leaf_params.push_str(&format!("{local_name}: {local_type}"));
            arguments.push_str(&local_name);
        }
        let result = emit_aggregate_return(
            &runtime_name,
            aggregate,
            return_type,
            &mut Vec::new(),
            &leaf_params,
            &arguments,
            &mut declaration,
        );
        let wrapper_key = format!("{package}::{}", function.name)
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let wrapper = format!("__thaw_jit_wrapper_{wrapper_key}");
        let wrapper_params = params
            .iter()
            .map(|(name, ty, optional)| {
                format!("{name}{}: {ty}", if *optional { "?" } else { "" })
            })
            .collect::<Vec<_>>()
            .join(", ");
        declaration.push_str(&format!(
            "function {wrapper}({wrapper_params}): {} {{ {local_statements}{result} }}\n",
            render_dynamic_type(return_type).unwrap()
        ));
        return (wrapper, declaration);
    }
    let JitExport::Value(operation) = operation else {
        unreachable!()
    };
    let runtime_key = format!("{operation}:{package}::{}", function.name);
    let symbol = encoded_symbol(&runtime_key);
    let union_ret = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements))
            if jit_tagged_union(elements) =>
        {
            render_dynamic_type(&thaw_hir::HirType::Union(elements.clone()))
        }
        _ => None,
    };
    let optional_union_ret = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload)) => {
            match payload.as_ref() {
                thaw_hir::HirType::Union(elements) if jit_tagged_union(elements) => {
                    render_dynamic_type(payload).map(|ty| format!("({ty}) | undefined"))
                }
                _ => None,
            }
        }
        _ => None,
    };
    let ret = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool) => "boolean",
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Str) => "string",
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements))
            if jit_tagged_union(elements) =>
        {
            union_ret.as_deref().unwrap()
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element)) => match element.as_ref() {
            thaw_hir::HirType::F64 => "number[]",
            thaw_hir::HirType::Bool => "boolean[]",
            thaw_hir::HirType::Str => "string[]",
            thaw_hir::HirType::Tuple(elements)
                if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::F64]) =>
            {
                "[string, number][]"
            }
            thaw_hir::HirType::Tuple(elements)
                if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::Bool]) =>
            {
                "[string, boolean][]"
            }
            thaw_hir::HirType::Tuple(elements)
                if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::Str]) =>
            {
                "[string, string][]"
            }
            _ => "never[]",
        },
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(element)) => {
            match element.as_ref() {
                thaw_hir::HirType::F64 => "{ [key: string]: number }",
                thaw_hir::HirType::Bool => "{ [key: string]: boolean }",
                thaw_hir::HirType::Str => "{ [key: string]: string }",
                _ => "never",
            }
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload))
            if **payload == thaw_hir::HirType::Str =>
        {
            "string | undefined"
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload))
            if **payload == thaw_hir::HirType::F64 =>
        {
            "number | undefined"
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload))
            if **payload == thaw_hir::HirType::Bool =>
        {
            "boolean | undefined"
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload))
            if matches!(payload.as_ref(), thaw_hir::HirType::Union(elements) if jit_tagged_union(elements)) =>
        {
            optional_union_ret.as_deref().unwrap()
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Optional(payload)) => {
            match payload.as_ref() {
                thaw_hir::HirType::Array(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => "number[] | undefined",
                    thaw_hir::HirType::Bool => "boolean[] | undefined",
                    thaw_hir::HirType::Str => "string[] | undefined",
                    thaw_hir::HirType::Tuple(elements)
                        if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::F64]) =>
                    {
                        "[string, number][] | undefined"
                    }
                    thaw_hir::HirType::Tuple(elements)
                        if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::Bool]) =>
                    {
                        "[string, boolean][] | undefined"
                    }
                    thaw_hir::HirType::Tuple(elements)
                        if matches!(elements.as_slice(), [thaw_hir::HirType::Str, thaw_hir::HirType::Str]) =>
                    {
                        "[string, string][] | undefined"
                    }
                    _ => "never",
                },
                thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => "{ [key: string]: number } | undefined",
                    thaw_hir::HirType::Bool => "{ [key: string]: boolean } | undefined",
                    thaw_hir::HirType::Str => "{ [key: string]: string } | undefined",
                    _ => "never",
                },
                _ => "never",
            }
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Nullable(payload)) => {
            match payload.as_ref() {
                thaw_hir::HirType::F64 => "number | null",
                thaw_hir::HirType::Bool => "boolean | null",
                thaw_hir::HirType::Str => "string | null",
                thaw_hir::HirType::Array(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => "number[] | null",
                    thaw_hir::HirType::Bool => "boolean[] | null",
                    thaw_hir::HirType::Str => "string[] | null",
                    _ => "never",
                },
                thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => "{ [key: string]: number } | null",
                    thaw_hir::HirType::Bool => "{ [key: string]: boolean } | null",
                    thaw_hir::HirType::Str => "{ [key: string]: string } | null",
                    _ => "never",
                },
                _ => "never",
            }
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Nullish(payload)) => {
            match payload.as_ref() {
                thaw_hir::HirType::F64 => "number | null | undefined",
                thaw_hir::HirType::Bool => "boolean | null | undefined",
                thaw_hir::HirType::Str => "string | null | undefined",
                thaw_hir::HirType::Array(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => "number[] | null | undefined",
                    thaw_hir::HirType::Bool => "boolean[] | null | undefined",
                    thaw_hir::HirType::Str => "string[] | null | undefined",
                    _ => "never",
                },
                thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => {
                        "{ [key: string]: number } | null | undefined"
                    }
                    thaw_hir::HirType::Bool => {
                        "{ [key: string]: boolean } | null | undefined"
                    }
                    thaw_hir::HirType::Str => {
                        "{ [key: string]: string } | null | undefined"
                    }
                    _ => "never",
                },
                _ => "never",
            }
        }
        _ => "number",
    };
    let mut declaration = format!("declare function {symbol}({direct_params}): {ret};\n");
    if function.required_params == params.len() {
        return (symbol, declaration);
    }
    let wrapper_key = format!("{package}::{}", function.name)
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let wrapper = format!("__thaw_jit_wrapper_{wrapper_key}");
    let wrapper_params = params
        .iter()
        .map(|(name, ty, optional)| {
            format!("{name}{}: {ty}", if *optional { "?" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = params
        .iter()
        .map(|(name, _, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    declaration.push_str(&format!(
        "function {wrapper}({wrapper_params}): {ret} {{ return {symbol}({arguments}); }}\n"
    ));
    (wrapper, declaration)
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
        thaw_hir::HirType::Union(elements) => elements
            .iter()
            .map(render_dynamic_type)
            .collect::<Option<Vec<_>>>()
            .map(|elements| elements.join(" | ")),
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
        thaw_hir::HirType::Dictionary(element) => {
            render_dynamic_type(element).map(|element| format!("{{ [key: string]: {element} }}"))
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
                        jit_export(
                            source,
                            &function.name,
                            pkg.commonjs_export_name.as_deref() == Some(&function.name)
                                || pkg.functions.len() == 1,
                            function,
                        )
                    });
                let declaration = jit_operation
                    .as_ref()
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
