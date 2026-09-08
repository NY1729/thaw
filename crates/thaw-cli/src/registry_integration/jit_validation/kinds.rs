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

fn jit_rejection_reason(
    source: &str,
    function: &thaw_bridge::DtsFunction,
) -> String {
    if function.generic.is_some() {
        return "generic function specialization is not available for this declaration".into();
    }
    if function.rest_param.is_some() {
        return "rest parameters are outside the specialization JIT ABI".into();
    }
    if let Some((name, ty)) = function.params.iter().find(|(_, ty)| {
        !matches!(ty, thaw_bridge::DtsType::Native(ty) if jit_diagnostic_type_supported(ty))
    }) {
        return format!("parameter `{name}` has unsupported JIT type {ty:?}");
    }
    if !matches!(&function.ret, thaw_bridge::DtsType::Native(ty) if jit_diagnostic_type_supported(ty))
    {
        return format!("return value has unsupported JIT type {:?}", function.ret);
    }
    if thaw_parser::parse_javascript(source).is_err() {
        return "package JavaScript could not be parsed for specialization".into();
    }
    "function body uses an expression, closure, external state, or control flow outside the specialization JIT IR".into()
}

fn jit_diagnostic_type_supported(ty: &thaw_hir::HirType) -> bool {
    match ty {
        thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str => true,
        thaw_hir::HirType::Array(element) | thaw_hir::HirType::Dictionary(element) => {
            matches!(element.as_ref(), thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str)
        }
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| jit_diagnostic_type_supported(field)),
        thaw_hir::HirType::Tuple(elements) | thaw_hir::HirType::Union(elements) => {
            elements.iter().all(jit_diagnostic_type_supported)
        }
        thaw_hir::HirType::Optional(payload)
        | thaw_hir::HirType::Nullable(payload)
        | thaw_hir::HirType::Nullish(payload) => jit_diagnostic_type_supported(payload),
        _ => false,
    }
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

fn jit_dynamic_array_result(tokens: &[String]) -> bool {
    let Some(token) = tokens.last().map(String::as_str) else {
        return false;
    };
    matches!(
        token,
        "dynarrayslice"
            | "dynarrayconcat"
            | "dynarrayappend"
            | "dynarrayreversed"
            | "dynarrayreverse"
            | "dynarraysorted"
            | "dynarraysort"
            | "dynarrayfill"
            | "dynarraycopywithin"
            | "dynarraywith"
            | "dynarraysplice"
            | "dynarraytospliced"
            | "dynarrayfiltertruthy"
            | "dynarraymaptonumber"
            | "dynarraymaptoboolean"
            | "dynarraymaptostring"
            | "dynarraymapidentity"
    ) || token.starts_with("dynarraymapjit")
}

fn jit_typed_array_union_untag(token: &str) -> Option<&'static str> {
    let (_, kinds) = jit_dynamic_argument(token)?;
    if kinds.bytes().all(|kind| matches!(kind, b'N' | b'X')) {
        Some("untagarrayn")
    } else if kinds.bytes().all(|kind| matches!(kind, b'B' | b'Y')) {
        Some("untagarrayb")
    } else if kinds.bytes().all(|kind| matches!(kind, b'S' | b'Z')) {
        Some("untagarrays")
    } else {
        None
    }
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
        if matches!(token.as_str(), "strarray" | "dkeys" | "split") {
            return Some("rs");
        }
        if token == "rsmaplength" {
            return Some("rn");
        }
        for (operation, prefix) in [
            ("maptonumber", "rn"),
            ("maptoboolean", "rb"),
            ("maptostring", "rs"),
        ] {
            if token
                .get(2..)
                .is_some_and(|suffix| suffix == operation)
                && matches!(token.get(..2), Some("rn" | "rb" | "rs"))
            {
                return Some(prefix);
            }
        }
        if let Some(target) = token
            .get(2..)
            .and_then(|suffix| suffix.strip_prefix("mapjit"))
            .filter(|_| matches!(token.get(..2), Some("rb" | "rs")))
        {
            return match target.as_bytes().first() {
                Some(b'n') => Some("rn"),
                Some(b'b') => Some("rb"),
                Some(b's') => Some("rs"),
                _ => None,
            };
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

