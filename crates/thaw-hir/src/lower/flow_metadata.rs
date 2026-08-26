impl<'a> FnLowerer<'a> {
    fn expression_union_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<HashMap<Symbol, Vec<Option<HirLit>>>> {
        match expression {
            Expr::Ident(identifier) => self
                .union_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::Paren(parenthesized) => self.expression_union_discriminants(&parenthesized.expr),
            Expr::TsSatisfies(assertion) => self.expression_union_discriminants(&assertion.expr),
            Expr::TsNonNull(assertion) => self.expression_union_discriminants(&assertion.expr),
            Expr::TsAs(assertion) => {
                let metadata =
                    object_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata =
                    object_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Await(awaited) => self.expression_union_discriminants(&awaited.arg),
            Expr::OptChain(chain) => {
                self.expression_union_discriminants(&ordinary_optional_chain_expression(chain))
            }
            Expr::New(construction) if matches!(construction.callee.as_ref(), Expr::Ident(name) if name.sym == *"Promise") =>
            {
                let metadata = construction
                    .type_args
                    .as_ref()
                    .and_then(|arguments| arguments.params.first())
                    .map(|ty| object_union_discriminants(ty, self.generic_interfaces))
                    .unwrap_or_default();
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Member(member) if matches!(member.prop, MemberProp::Computed(_)) => {
                self.expression_array_element_discriminants(&member.obj)
            }
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let callee = ordinary_optional_expression(callee);
                if let Expr::Member(member) = &callee {
                    let property = member_property_name(&member.prop)?;
                    if property == "finally" {
                        return self.expression_union_discriminants(&member.obj);
                    }
                    if property == "then" {
                        return call.args.first().and_then(|argument| {
                            self.expression_function_discriminants(&argument.expr)
                        });
                    }
                    if property == "catch" {
                        let source = self.expression_union_discriminants(&member.obj);
                        let recovered = call.args.first().and_then(|argument| {
                            self.expression_function_discriminants(&argument.expr)
                        });
                        return match (source, recovered) {
                            (Some(source), Some(recovered)) if source == recovered => Some(source),
                            (Some(source), None) => Some(source),
                            (None, Some(recovered)) => Some(recovered),
                            _ => None,
                        };
                    }
                    if matches!(property.as_str(), "race" | "any")
                        && matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Promise")
                    {
                        return call.args.first().and_then(|argument| {
                            self.expression_array_element_discriminants(&argument.expr)
                        });
                    }
                    if matches!(property.as_str(), "at" | "find" | "findLast") {
                        return self.expression_array_element_discriminants(&member.obj);
                    }
                    if matches!(property.as_str(), "reduce" | "reduceRight") {
                        return call
                            .args
                            .get(1)
                            .and_then(|argument| {
                                self.expression_union_discriminants(&argument.expr)
                            })
                            .or_else(|| self.expression_array_element_discriminants(&member.obj));
                    }
                    return self
                        .expression_called_function_property_discriminants(&callee)
                        .and_then(|metadata| metadata.value);
                }
                let Expr::Ident(callee) = &callee else {
                    return None;
                };
                self.function_value_discriminants
                    .get(&self.resolve_binding(callee.sym.as_ref()))
                    .or_else(|| {
                        self.generic_interfaces
                            .function_discriminants
                            .get(callee.sym.as_ref())
                    })
                    .cloned()
            }
            Expr::Cond(conditional) => {
                let consequent = self.expression_union_discriminants(&conditional.cons)?;
                (self.expression_union_discriminants(&conditional.alt)? == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_array_element_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<HashMap<Symbol, Vec<Option<HirLit>>>> {
        match expression {
            Expr::Ident(identifier) => self
                .array_element_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_array_element_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_array_element_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_array_element_discriminants(&assertion.expr)
            }
            Expr::TsAs(assertion) => {
                let metadata =
                    array_element_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata =
                    array_element_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_array_element_discriminants(&assertion.expr)
            }
            Expr::Await(awaited) => self.expression_array_element_discriminants(&awaited.arg),
            Expr::OptChain(chain) => self
                .expression_array_element_discriminants(&ordinary_optional_chain_expression(chain)),
            Expr::Member(member) => {
                let property = member_property_name(&member.prop)?;
                self.expression_object_array_property_discriminants(&member.obj)?
                    .get(&vec![property])
                    .cloned()
            }
            Expr::Array(array) => {
                let mut merged = None;
                for element in array.elems.iter().flatten() {
                    let metadata = if element.spread.is_some() {
                        self.expression_array_element_discriminants(&element.expr)
                    } else {
                        self.expression_union_discriminants(&element.expr)
                    }?;
                    if let Some(previous) = &merged {
                        if previous != &metadata {
                            return None;
                        }
                    } else {
                        merged = Some(metadata);
                    }
                }
                merged
            }
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let callee = ordinary_optional_expression(callee);
                if let Expr::Member(member) = &callee {
                    let property = member_property_name(&member.prop)?;
                    if property == "flat" {
                        let depth = match call.args.first().map(|argument| argument.expr.as_ref()) {
                            None => 1,
                            Some(Expr::Lit(Lit::Num(value))) if value.value <= 0.0 => 0,
                            Some(Expr::Lit(Lit::Num(value))) => value.value.trunc() as usize,
                            _ => return None,
                        };
                        return self
                            .expression_nested_array_discriminants(&member.obj)
                            .and_then(|metadata| metadata.get(&(depth + 1)).cloned());
                    }
                    if property == "all"
                        && matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Promise")
                    {
                        return call.args.first().and_then(|argument| {
                            self.expression_array_element_discriminants(&argument.expr)
                        });
                    }
                    if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Array")
                    {
                        if property == "of" {
                            let metadata = call
                                .type_args
                                .as_ref()
                                .and_then(|arguments| arguments.params.first())
                                .map(|ty| object_union_discriminants(ty, self.generic_interfaces))
                                .unwrap_or_default();
                            return (!metadata.is_empty()).then_some(metadata);
                        }
                        if property == "from" {
                            if call.args.len() == 1 {
                                return self
                                    .expression_array_element_discriminants(&call.args[0].expr);
                            }
                            let explicit_output = call.type_args.as_ref().and_then(|arguments| {
                                arguments.params.get(1).or_else(|| arguments.params.first())
                            });
                            if let Some(output) = explicit_output {
                                let metadata =
                                    object_union_discriminants(output, self.generic_interfaces);
                                if !metadata.is_empty() {
                                    return Some(metadata);
                                }
                            }
                            return call.args.get(1).and_then(|argument| {
                                self.expression_function_discriminants(&argument.expr)
                            });
                        }
                    }
                    if property == "map" {
                        return call.args.first().and_then(|argument| {
                            self.expression_function_discriminants(&argument.expr)
                        });
                    }
                    if property == "flatMap" {
                        return call.args.first().and_then(|argument| {
                            self.expression_function_array_discriminants(&argument.expr)
                        });
                    }
                    if matches!(
                        property.as_str(),
                        "concat"
                            | "copyWithin"
                            | "fill"
                            | "filter"
                            | "reverse"
                            | "slice"
                            | "sort"
                            | "toSorted"
                            | "toReversed"
                            | "toSpliced"
                            | "with"
                    ) {
                        if let Some(discriminants) =
                            self.expression_array_element_discriminants(&member.obj)
                        {
                            return Some(discriminants);
                        }
                    }
                    return self
                        .expression_called_function_property_discriminants(&callee)
                        .and_then(|metadata| metadata.array);
                }
                let Expr::Ident(callee) = &callee else {
                    return None;
                };
                self.function_value_array_discriminants
                    .get(&self.resolve_binding(callee.sym.as_ref()))
                    .or_else(|| {
                        self.generic_interfaces
                            .function_array_discriminants
                            .get(callee.sym.as_ref())
                    })
                    .cloned()
            }
            Expr::Cond(conditional) => {
                let consequent = self.expression_array_element_discriminants(&conditional.cons)?;
                (self.expression_array_element_discriminants(&conditional.alt)? == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_nested_array_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<NestedArrayDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .nested_array_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_nested_array_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_nested_array_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_nested_array_discriminants(&assertion.expr)
            }
            Expr::TsAs(assertion) => {
                let metadata =
                    nested_array_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata =
                    nested_array_union_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_nested_array_discriminants(&assertion.expr)
            }
            Expr::Await(awaited) => self.expression_nested_array_discriminants(&awaited.arg),
            Expr::OptChain(chain) => self
                .expression_nested_array_discriminants(&ordinary_optional_chain_expression(chain)),
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let callee = ordinary_optional_expression(callee);
                let Expr::Member(member) = &callee else {
                    let Expr::Ident(callee) = &callee else {
                        return None;
                    };
                    return self
                        .function_value_nested_array_discriminants
                        .get(&self.resolve_binding(callee.sym.as_ref()))
                        .or_else(|| {
                            self.generic_interfaces
                                .function_nested_array_discriminants
                                .get(callee.sym.as_ref())
                        })
                        .cloned();
                };
                let property = member_property_name(&member.prop)?;
                if property == "flat" {
                    let depth = match call.args.first().map(|argument| argument.expr.as_ref()) {
                        None => 1,
                        Some(Expr::Lit(Lit::Num(value))) if value.value <= 0.0 => 0,
                        Some(Expr::Lit(Lit::Num(value))) => value.value.trunc() as usize,
                        _ => return None,
                    };
                    let metadata = self.expression_nested_array_discriminants(&member.obj)?;
                    let shifted = metadata
                        .into_iter()
                        .filter_map(|(level, discriminants)| {
                            (level > depth).then_some((level - depth, discriminants))
                        })
                        .collect::<NestedArrayDiscriminants>();
                    return (!shifted.is_empty()).then_some(shifted);
                }
                if matches!(
                    property.as_str(),
                    "concat"
                        | "copyWithin"
                        | "fill"
                        | "filter"
                        | "reverse"
                        | "slice"
                        | "sort"
                        | "toSorted"
                        | "toReversed"
                        | "toSpliced"
                        | "with"
                ) {
                    return self.expression_nested_array_discriminants(&member.obj);
                }
                self.expression_called_function_property_discriminants(&callee)
                    .and_then(|metadata| {
                        (!metadata.nested_array.is_empty()).then_some(metadata.nested_array)
                    })
            }
            _ => None,
        }
    }

    fn expression_function_nested_array_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<NestedArrayDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .function_value_nested_array_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .or_else(|| {
                    self.generic_interfaces
                        .function_nested_array_discriminants
                        .get(identifier.sym.as_ref())
                })
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_function_nested_array_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_function_nested_array_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_function_nested_array_discriminants(&assertion.expr)
            }
            Expr::OptChain(chain) => self.expression_function_nested_array_discriminants(
                &ordinary_optional_chain_expression(chain),
            ),
            Expr::Member(_) => self
                .expression_called_function_property_discriminants(expression)
                .and_then(|metadata| {
                    (!metadata.nested_array.is_empty()).then_some(metadata.nested_array)
                }),
            Expr::TsAs(assertion) => {
                let metadata = function_return_nested_array_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = function_return_nested_array_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Arrow(arrow) => arrow.return_type.as_ref().and_then(|annotation| {
                let metadata =
                    nested_array_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }),
            Expr::Fn(function) => function
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| {
                    let metadata = nested_array_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    (!metadata.is_empty()).then_some(metadata)
                }),
            Expr::Cond(conditional) => {
                let consequent =
                    self.expression_function_nested_array_discriminants(&conditional.cons)?;
                (self.expression_function_nested_array_discriminants(&conditional.alt)?
                    == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_object_array_property_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<ObjectArrayPropertyDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .object_array_property_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_object_array_property_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_object_array_property_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_object_array_property_discriminants(&assertion.expr)
            }
            Expr::TsAs(assertion) => {
                let metadata = object_array_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = object_array_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_object_array_property_discriminants(&assertion.expr)
            }
            Expr::Await(awaited) => {
                self.expression_object_array_property_discriminants(&awaited.arg)
            }
            Expr::OptChain(chain) => self.expression_object_array_property_discriminants(
                &ordinary_optional_chain_expression(chain),
            ),
            Expr::Member(member) => {
                let property = member_property_name(&member.prop)?;
                let nested = self
                    .expression_object_array_property_discriminants(&member.obj)?
                    .into_iter()
                    .filter_map(|(path, discriminants)| {
                        (path.first() == Some(&property) && path.len() > 1)
                            .then(|| (path[1..].to_vec(), discriminants))
                    })
                    .collect::<ObjectArrayPropertyDiscriminants>();
                (!nested.is_empty()).then_some(nested)
            }
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let callee = ordinary_optional_expression(callee);
                if matches!(&callee, Expr::Member(_)) {
                    let metadata = self
                        .expression_called_function_property_discriminants(&callee)?
                        .object;
                    return (!metadata.is_empty()).then_some(metadata);
                }
                let Expr::Ident(callee) = &callee else {
                    return None;
                };
                self.function_value_object_array_property_discriminants
                    .get(&self.resolve_binding(callee.sym.as_ref()))
                    .or_else(|| {
                        self.generic_interfaces
                            .function_object_array_property_discriminants
                            .get(callee.sym.as_ref())
                    })
                    .cloned()
            }
            Expr::Object(object) => self.object_literal_array_property_discriminants(object),
            Expr::Cond(conditional) => {
                let consequent =
                    self.expression_object_array_property_discriminants(&conditional.cons)?;
                (self.expression_object_array_property_discriminants(&conditional.alt)?
                    == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn object_literal_array_property_discriminants(
        &self,
        object: &SwcObjectLit,
    ) -> Option<ObjectArrayPropertyDiscriminants> {
        let mut metadata = ObjectArrayPropertyDiscriminants::new();
        for property in &object.props {
            match property {
                PropOrSpread::Spread(spread) => {
                    let Some(incoming) =
                        self.expression_object_array_property_discriminants(&spread.expr)
                    else {
                        continue;
                    };
                    for first in incoming.keys().filter_map(|path| path.first()) {
                        metadata.retain(|path, _| path.first() != Some(first));
                    }
                    metadata.extend(incoming);
                }
                PropOrSpread::Prop(property) => {
                    let (name, value) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let Ok(name) = class_property_name(&property.key) else {
                                continue;
                            };
                            (name, Some(property.value.as_ref()))
                        }
                        Prop::Shorthand(identifier) => {
                            let name = identifier.sym.to_string();
                            metadata.retain(|path, _| path.first() != Some(&name));
                            let binding = self.resolve_binding(identifier.sym.as_ref());
                            if let Some(discriminants) =
                                self.array_element_discriminants.get(&binding)
                            {
                                metadata.insert(vec![name.clone()], discriminants.clone());
                            }
                            if let Some(nested) =
                                self.object_array_property_discriminants.get(&binding)
                            {
                                metadata.extend(nested.iter().map(|(path, value)| {
                                    let mut path = path.clone();
                                    path.insert(0, name.clone());
                                    (path, value.clone())
                                }));
                            }
                            (name, None)
                        }
                        _ => continue,
                    };
                    let Some(value) = value else {
                        continue;
                    };
                    metadata.retain(|path, _| path.first() != Some(&name));
                    if let Some(discriminants) = self.expression_array_element_discriminants(value)
                    {
                        metadata.insert(vec![name.clone()], discriminants);
                    }
                    if let Some(nested) = self.expression_object_array_property_discriminants(value)
                    {
                        metadata.extend(nested.into_iter().map(|(mut path, value)| {
                            path.insert(0, name.clone());
                            (path, value)
                        }));
                    }
                }
            }
        }
        (!metadata.is_empty()).then_some(metadata)
    }

    fn expression_object_function_property_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<ObjectFunctionPropertyDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .object_function_property_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_object_function_property_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_object_function_property_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_object_function_property_discriminants(&assertion.expr)
            }
            Expr::TsAs(assertion) => {
                let metadata = object_function_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = object_function_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_object_function_property_discriminants(&assertion.expr)
            }
            Expr::Await(awaited) => {
                self.expression_object_function_property_discriminants(&awaited.arg)
            }
            Expr::OptChain(chain) => self.expression_object_function_property_discriminants(
                &ordinary_optional_chain_expression(chain),
            ),
            Expr::Member(member) => {
                let property = member_property_name(&member.prop)?;
                let nested = self
                    .expression_object_function_property_discriminants(&member.obj)?
                    .into_iter()
                    .filter_map(|(path, discriminants)| {
                        (path.first() == Some(&property) && path.len() > 1)
                            .then(|| (path[1..].to_vec(), discriminants))
                    })
                    .collect::<ObjectFunctionPropertyDiscriminants>();
                (!nested.is_empty()).then_some(nested)
            }
            Expr::Object(object) => self.object_literal_function_property_discriminants(object),
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let callee = ordinary_optional_expression(callee);
                if matches!(&callee, Expr::Member(_)) {
                    let metadata = self
                        .expression_called_function_property_discriminants(&callee)?
                        .functions;
                    return (!metadata.is_empty()).then_some(metadata);
                }
                let Expr::Ident(callee) = &callee else {
                    return None;
                };
                self.function_value_object_function_property_discriminants
                    .get(&self.resolve_binding(callee.sym.as_ref()))
                    .or_else(|| {
                        self.generic_interfaces
                            .function_object_function_property_discriminants
                            .get(callee.sym.as_ref())
                    })
                    .cloned()
            }
            Expr::Cond(conditional) => {
                let consequent =
                    self.expression_object_function_property_discriminants(&conditional.cons)?;
                (self.expression_object_function_property_discriminants(&conditional.alt)?
                    == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_function_property_result_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<FunctionPropertyDiscriminants> {
        let value = self.expression_function_discriminants(expression);
        let array = self.expression_function_array_discriminants(expression);
        let nested_array = self
            .expression_function_nested_array_discriminants(expression)
            .unwrap_or_default();
        let object = self
            .expression_function_object_array_property_discriminants(expression)
            .unwrap_or_default();
        let functions = self
            .expression_function_object_function_property_discriminants(expression)
            .unwrap_or_default();
        (value.is_some()
            || array.is_some()
            || !nested_array.is_empty()
            || !object.is_empty()
            || !functions.is_empty())
        .then_some(FunctionPropertyDiscriminants {
            value,
            array,
            nested_array,
            object,
            functions,
        })
    }

    fn insert_object_literal_function_property(
        &self,
        metadata: &mut ObjectFunctionPropertyDiscriminants,
        name: Symbol,
        value: &Expr,
    ) {
        metadata.retain(|path, _| path.first() != Some(&name));
        if let Some(discriminants) = self.expression_function_property_result_discriminants(value) {
            metadata.insert(vec![name.clone()], discriminants);
        }
        if let Some(nested) = self.expression_object_function_property_discriminants(value) {
            metadata.extend(nested.into_iter().map(|(mut path, discriminants)| {
                path.insert(0, name.clone());
                (path, discriminants)
            }));
        }
    }

    fn object_literal_function_property_discriminants(
        &self,
        object: &SwcObjectLit,
    ) -> Option<ObjectFunctionPropertyDiscriminants> {
        let mut metadata = ObjectFunctionPropertyDiscriminants::new();
        for property in &object.props {
            match property {
                PropOrSpread::Spread(spread) => {
                    let Some(incoming) =
                        self.expression_object_function_property_discriminants(&spread.expr)
                    else {
                        continue;
                    };
                    for first in incoming.keys().filter_map(|path| path.first()) {
                        metadata.retain(|path, _| path.first() != Some(first));
                    }
                    metadata.extend(incoming);
                }
                PropOrSpread::Prop(property) => match property.as_ref() {
                    Prop::KeyValue(property) => {
                        let Ok(name) = class_property_name(&property.key) else {
                            continue;
                        };
                        self.insert_object_literal_function_property(
                            &mut metadata,
                            name,
                            &property.value,
                        );
                    }
                    Prop::Shorthand(identifier) => {
                        self.insert_object_literal_function_property(
                            &mut metadata,
                            identifier.sym.to_string(),
                            &Expr::Ident(identifier.clone()),
                        );
                    }
                    _ => {}
                },
            }
        }
        (!metadata.is_empty()).then_some(metadata)
    }

    fn expression_called_function_property_discriminants(
        &self,
        callee: &Expr,
    ) -> Option<FunctionPropertyDiscriminants> {
        let Expr::Member(member) = callee else {
            return None;
        };
        let property = member_property_name(&member.prop)?;
        self.expression_object_function_property_discriminants(&member.obj)?
            .get(std::slice::from_ref(&property))
            .cloned()
    }

    fn hir_array_element_discriminants(&self, expression: &HirExpr) -> Option<UnionDiscriminants> {
        match expression {
            HirExpr::Var(name) => self.array_element_discriminants.get(name).cloned(),
            HirExpr::PropAccess(object, _, property) => self
                .hir_object_array_property_discriminants(object)?
                .get(std::slice::from_ref(property))
                .cloned(),
            _ => None,
        }
    }

    fn hir_object_array_property_discriminants(
        &self,
        expression: &HirExpr,
    ) -> Option<ObjectArrayPropertyDiscriminants> {
        match expression {
            HirExpr::Var(name) => self.object_array_property_discriminants.get(name).cloned(),
            HirExpr::ObjectLit(fields) => {
                let mut metadata = ObjectArrayPropertyDiscriminants::new();
                for (name, value) in fields {
                    if let Some(discriminants) = self.hir_array_element_discriminants(value) {
                        metadata.insert(vec![name.clone()], discriminants);
                    }
                    if let Some(nested) = self.hir_object_array_property_discriminants(value) {
                        metadata.extend(nested.into_iter().map(|(mut path, discriminants)| {
                            path.insert(0, name.clone());
                            (path, discriminants)
                        }));
                    }
                }
                (!metadata.is_empty()).then_some(metadata)
            }
            HirExpr::PropAccess(object, _, property) => {
                let nested = self
                    .hir_object_array_property_discriminants(object)?
                    .into_iter()
                    .filter_map(|(path, discriminants)| {
                        (path.first() == Some(property) && path.len() > 1)
                            .then(|| (path[1..].to_vec(), discriminants))
                    })
                    .collect::<ObjectArrayPropertyDiscriminants>();
                (!nested.is_empty()).then_some(nested)
            }
            _ => None,
        }
    }

    fn hir_function_property_discriminants(
        &self,
        expression: &HirExpr,
    ) -> Option<FunctionPropertyDiscriminants> {
        let HirExpr::PropAccess(object, _, property) = expression else {
            return None;
        };
        self.hir_object_function_property_discriminants(object)?
            .get(std::slice::from_ref(property))
            .cloned()
    }

    fn hir_object_function_property_discriminants(
        &self,
        expression: &HirExpr,
    ) -> Option<ObjectFunctionPropertyDiscriminants> {
        match expression {
            HirExpr::Var(name) => self
                .object_function_property_discriminants
                .get(name)
                .cloned(),
            HirExpr::ObjectLit(fields) => {
                let mut metadata = ObjectFunctionPropertyDiscriminants::new();
                for (name, value) in fields {
                    if let Some(discriminants) = self.hir_function_property_discriminants(value) {
                        metadata.insert(vec![name.clone()], discriminants);
                    }
                    if let Some(nested) = self.hir_object_function_property_discriminants(value) {
                        metadata.extend(nested.into_iter().map(|(mut path, discriminants)| {
                            path.insert(0, name.clone());
                            (path, discriminants)
                        }));
                    }
                }
                (!metadata.is_empty()).then_some(metadata)
            }
            HirExpr::PropAccess(object, _, property) => {
                let nested = self
                    .hir_object_function_property_discriminants(object)?
                    .into_iter()
                    .filter_map(|(path, discriminants)| {
                        (path.first() == Some(property) && path.len() > 1)
                            .then(|| (path[1..].to_vec(), discriminants))
                    })
                    .collect::<ObjectFunctionPropertyDiscriminants>();
                (!nested.is_empty()).then_some(nested)
            }
            _ => None,
        }
    }

    fn expression_identifier_alias_source(&self, expression: &Expr) -> Option<Symbol> {
        match expression {
            Expr::Ident(identifier) => Some(self.resolve_binding(identifier.sym.as_ref())),
            Expr::Paren(parenthesized) => {
                self.expression_identifier_alias_source(&parenthesized.expr)
            }
            Expr::TsAs(assertion) => self.expression_identifier_alias_source(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => {
                self.expression_identifier_alias_source(&assertion.expr)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_identifier_alias_source(&assertion.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_identifier_alias_source(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => self.expression_identifier_alias_source(&assertion.expr),
            _ => None,
        }
    }

    fn expression_static_property_path(&self, expression: &Expr) -> Option<(Symbol, Vec<Symbol>)> {
        match expression {
            Expr::Ident(identifier) => {
                Some((self.resolve_binding(identifier.sym.as_ref()), Vec::new()))
            }
            Expr::Member(member) => {
                let (root, mut path) = self.expression_static_property_path(&member.obj)?;
                path.push(member_property_name(&member.prop)?);
                Some((root, path))
            }
            Expr::Paren(parenthesized) => self.expression_static_property_path(&parenthesized.expr),
            Expr::TsAs(assertion) => self.expression_static_property_path(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => {
                self.expression_static_property_path(&assertion.expr)
            }
            Expr::TsConstAssertion(assertion) => {
                self.expression_static_property_path(&assertion.expr)
            }
            Expr::TsSatisfies(assertion) => self.expression_static_property_path(&assertion.expr),
            Expr::TsNonNull(assertion) => self.expression_static_property_path(&assertion.expr),
            _ => None,
        }
    }

    fn expression_function_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<HashMap<Symbol, Vec<Option<HirLit>>>> {
        match expression {
            Expr::Ident(identifier) => self
                .function_value_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .or_else(|| {
                    self.generic_interfaces
                        .function_discriminants
                        .get(identifier.sym.as_ref())
                })
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_function_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => self.expression_function_discriminants(&assertion.expr),
            Expr::TsNonNull(assertion) => self.expression_function_discriminants(&assertion.expr),
            Expr::OptChain(chain) => {
                self.expression_function_discriminants(&ordinary_optional_chain_expression(chain))
            }
            Expr::Member(_) => self
                .expression_called_function_property_discriminants(expression)
                .and_then(|metadata| metadata.value),
            Expr::TsAs(assertion) => {
                let metadata =
                    function_return_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata =
                    function_return_discriminants(&assertion.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Arrow(arrow) => arrow.return_type.as_ref().and_then(|annotation| {
                let metadata =
                    object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                (!metadata.is_empty()).then_some(metadata)
            }),
            Expr::Fn(function) => function
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| {
                    let metadata =
                        object_union_discriminants(&annotation.type_ann, self.generic_interfaces);
                    (!metadata.is_empty()).then_some(metadata)
                }),
            Expr::Cond(conditional) => {
                let consequent = self.expression_function_discriminants(&conditional.cons)?;
                (self.expression_function_discriminants(&conditional.alt)? == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_function_array_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<HashMap<Symbol, Vec<Option<HirLit>>>> {
        match expression {
            Expr::Ident(identifier) => self
                .function_value_array_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .or_else(|| {
                    self.generic_interfaces
                        .function_array_discriminants
                        .get(identifier.sym.as_ref())
                })
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_function_array_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_function_array_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_function_array_discriminants(&assertion.expr)
            }
            Expr::OptChain(chain) => self.expression_function_array_discriminants(
                &ordinary_optional_chain_expression(chain),
            ),
            Expr::Member(_) => self
                .expression_called_function_property_discriminants(expression)
                .and_then(|metadata| metadata.array),
            Expr::TsAs(assertion) => {
                let metadata = function_return_array_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = function_return_array_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Arrow(arrow) => arrow.return_type.as_ref().and_then(|annotation| {
                let metadata = array_element_union_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }),
            Expr::Fn(function) => function
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| {
                    let metadata = array_element_union_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    (!metadata.is_empty()).then_some(metadata)
                }),
            Expr::Cond(conditional) => {
                let consequent = self.expression_function_array_discriminants(&conditional.cons)?;
                (self.expression_function_array_discriminants(&conditional.alt)? == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_function_object_array_property_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<ObjectArrayPropertyDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .function_value_object_array_property_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .or_else(|| {
                    self.generic_interfaces
                        .function_object_array_property_discriminants
                        .get(identifier.sym.as_ref())
                })
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_function_object_array_property_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_function_object_array_property_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_function_object_array_property_discriminants(&assertion.expr)
            }
            Expr::OptChain(chain) => self.expression_function_object_array_property_discriminants(
                &ordinary_optional_chain_expression(chain),
            ),
            Expr::Member(_) => self
                .expression_called_function_property_discriminants(expression)
                .and_then(|metadata| (!metadata.object.is_empty()).then_some(metadata.object)),
            Expr::TsAs(assertion) => {
                let metadata = function_return_object_array_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = function_return_object_array_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Arrow(arrow) => arrow.return_type.as_ref().and_then(|annotation| {
                let metadata = object_array_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }),
            Expr::Fn(function) => function
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| {
                    let metadata = object_array_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    (!metadata.is_empty()).then_some(metadata)
                }),
            Expr::Cond(conditional) => {
                let consequent = self
                    .expression_function_object_array_property_discriminants(&conditional.cons)?;
                (self.expression_function_object_array_property_discriminants(&conditional.alt)?
                    == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

    fn expression_function_object_function_property_discriminants(
        &self,
        expression: &Expr,
    ) -> Option<ObjectFunctionPropertyDiscriminants> {
        match expression {
            Expr::Ident(identifier) => self
                .function_value_object_function_property_discriminants
                .get(&self.resolve_binding(identifier.sym.as_ref()))
                .or_else(|| {
                    self.generic_interfaces
                        .function_object_function_property_discriminants
                        .get(identifier.sym.as_ref())
                })
                .cloned(),
            Expr::Paren(parenthesized) => {
                self.expression_function_object_function_property_discriminants(&parenthesized.expr)
            }
            Expr::TsSatisfies(assertion) => {
                self.expression_function_object_function_property_discriminants(&assertion.expr)
            }
            Expr::TsNonNull(assertion) => {
                self.expression_function_object_function_property_discriminants(&assertion.expr)
            }
            Expr::OptChain(chain) => self
                .expression_function_object_function_property_discriminants(
                    &ordinary_optional_chain_expression(chain),
                ),
            Expr::Member(_) => self
                .expression_called_function_property_discriminants(expression)
                .and_then(|metadata| {
                    (!metadata.functions.is_empty()).then_some(metadata.functions)
                }),
            Expr::TsAs(assertion) => {
                let metadata = function_return_object_function_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::TsTypeAssertion(assertion) => {
                let metadata = function_return_object_function_property_discriminants(
                    &assertion.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }
            Expr::Arrow(arrow) => arrow.return_type.as_ref().and_then(|annotation| {
                let metadata = object_function_property_discriminants(
                    &annotation.type_ann,
                    self.generic_interfaces,
                );
                (!metadata.is_empty()).then_some(metadata)
            }),
            Expr::Fn(function) => function
                .function
                .return_type
                .as_ref()
                .and_then(|annotation| {
                    let metadata = object_function_property_discriminants(
                        &annotation.type_ann,
                        self.generic_interfaces,
                    );
                    (!metadata.is_empty()).then_some(metadata)
                }),
            Expr::Cond(conditional) => {
                let consequent = self.expression_function_object_function_property_discriminants(
                    &conditional.cons,
                )?;
                (self
                    .expression_function_object_function_property_discriminants(&conditional.alt)?
                    == consequent)
                    .then_some(consequent)
            }
            _ => None,
        }
    }

}
