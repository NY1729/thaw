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
