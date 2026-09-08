macro_rules! jit_aggregates {
    () => {
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
    };
}
