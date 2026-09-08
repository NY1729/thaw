fn napi_constructor_export_name(symbol: &str) -> Option<&str> {
    let constructor = symbol.strip_prefix("$new$")?;
    Some(
        constructor
            .rsplit_once("$arity")
            .map_or(constructor, |(name, _)| name),
    )
}

fn dynamic_member_name(symbol: &str, overloaded: bool) -> Option<String> {
    let member = symbol.splitn(4, '$').nth(3)?;
    Some(if overloaded {
        member.rsplit_once("$overload")?.0.to_string()
    } else {
        member.to_string()
    })
}

fn dynamic_json_collection_element_supported(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Str | HirType::Bool | HirType::Json => true,
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            dynamic_json_collection_element_supported(payload)
        }
        HirType::Array(element) => dynamic_json_collection_element_supported(element),
        HirType::Tuple(elements) => elements
            .iter()
            .all(dynamic_json_collection_element_supported),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| dynamic_json_collection_element_supported(field)),
        _ => false,
    }
}

fn quickjs_callback_type(ty: &HirType) -> bool {
    match ty {
        HirType::Function(_, _) | HirType::CallableFunction(..) => true,
        HirType::Optional(inner) => quickjs_callback_type(inner),
        _ => false,
    }
}

type NapiFunctionArgument = (usize, Vec<HirType>, HirType, bool, Option<usize>);

fn jit_parameter_slots(ty: &HirType) -> Option<usize> {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str => Some(1),
        HirType::Union(elements) if jit_argument_tagged_union(elements) => Some(2),
        HirType::Array(element)
            if jit_array_result_element_supported(element) =>
        {
            Some(1)
        }
        HirType::Dictionary(element)
            if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str) =>
        {
            Some(1)
        }
        HirType::Object(fields) => fields.iter().try_fold(0usize, |slots, (_, ty)| {
            jit_parameter_slots(ty).map(|count| slots + count)
        }),
        HirType::Tuple(elements) => elements.iter().try_fold(0usize, |slots, ty| {
            jit_parameter_slots(ty).map(|count| slots + count)
        }),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            jit_parameter_slots(payload).map(|slots| slots + 1)
        }
        _ => None,
    }
}

fn jit_array_result_element_supported(ty: &HirType) -> bool {
    matches!(ty, HirType::F64 | HirType::Bool | HirType::Str)
        || matches!(
            ty,
            HirType::Tuple(elements)
                if matches!(elements.as_slice(), [HirType::Str, HirType::F64 | HirType::Bool | HirType::Str])
        )
}

fn jit_tagged_union(elements: &[HirType]) -> bool {
    jit_argument_tagged_union(elements)
}

fn jit_argument_tagged_union(elements: &[HirType]) -> bool {
    (2..=9).contains(&elements.len())
        && elements.iter().all(|element| jit_union_member_tag(element).is_some())
        && elements
            .iter()
            .enumerate()
            .all(|(index, element)| !elements[..index].contains(element))
        && elements
            .iter()
            .filter(|element| matches!(element, HirType::Object(_)))
            .count()
            <= 1
        && elements
            .iter()
            .filter(|element| matches!(element, HirType::Tuple(_)))
            .count()
            <= 1
        && (elements.contains(&HirType::Str)
            || elements
                .iter()
                .any(|element| {
                    matches!(
                        element,
                        HirType::Array(_)
                            | HirType::Dictionary(_)
                            | HirType::Object(_)
                            | HirType::Tuple(_)
                    )
                }))
}

fn jit_union_member_tag(ty: &HirType) -> Option<u64> {
    match ty {
        HirType::F64 => Some(1),
        HirType::Str => Some(2),
        HirType::Bool => Some(3),
        HirType::Array(element) => match element.as_ref() {
            HirType::F64 => Some(4),
            HirType::Bool => Some(5),
            HirType::Str => Some(6),
            _ => None,
        },
        HirType::Dictionary(element) => match element.as_ref() {
            HirType::F64 => Some(7),
            HirType::Bool => Some(8),
            HirType::Str => Some(9),
            _ => None,
        },
        HirType::Object(_) => Some(10),
        HirType::Tuple(_) => Some(11),
        _ => None,
    }
}

