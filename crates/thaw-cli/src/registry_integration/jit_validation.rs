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

fn jit_expression_kind(expression: &[String]) -> Option<(JitKind, usize)> {
    let mut stack = Vec::new();
    let mut branches = Vec::new();
    let mut loops = Vec::new();
    let mut guards = Vec::new();
    let mut switches = Vec::new();
    let mut tries = Vec::new();
    let mut catches = Vec::new();
    let mut result_regions: Vec<(Vec<JitKind>, Option<Vec<JitKind>>)> = Vec::new();
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
        } else if token == "break"
            || token.starts_with("break")
            || token == "continue"
            || token.starts_with("continue")
        {
            let prefix = if token.starts_with("break") {
                "break"
            } else {
                "continue"
            };
            let encoded_depth = token.strip_prefix(prefix)?;
            let target_depth = if encoded_depth.is_empty() {
                0
            } else {
                encoded_depth.parse::<usize>().ok()?
            };
            let target = loops
                .len()
                .checked_sub(target_depth.checked_add(1)?)?;
            if !stack.starts_with(loops.get(target)?) {
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
            if !stack.starts_with(&base) {
                return None;
            }
            let results = stack.split_off(base.len());
            catches.push((base.clone(), results));
            stack = base;
            if tagged {
                stack.push(JitKind::Number);
                dynamic_slots.insert(stack.len());
                stack.push(JitKind::Number);
            } else {
                stack.push(kind?);
            }
        } else if token == "tryend" {
            let (base, expected) = catches.pop()?;
            if !stack.starts_with(&base) || stack.len() != base.len() + expected.len() {
                return None;
            }
            let caught = stack.split_off(base.len());
            for (expected, caught) in expected.into_iter().zip(caught) {
                stack.push(merge_jit_kinds(expected, caught)?);
            }
        } else if token == "resultstart" {
            result_regions.push((stack.clone(), None));
        } else if token == "resultreturn" || token.starts_with("resultreturn") {
            let continuation_depth = loops
                .last()
                .map(Vec::len)
                .into_iter()
                .chain(guards.last().map(|guard| guard.0.len()))
                .chain(switches.last().map(Vec::len))
                .max();
            let (base, expected) = result_regions.last_mut()?;
            if !stack.starts_with(base.as_slice()) || stack.len() == base.len() {
                return None;
            }
            let count = token
                .strip_prefix("resultreturn")
                .filter(|count| !count.is_empty())
                .map(str::parse::<usize>)
                .transpose()
                .ok()?
                .unwrap_or_else(|| stack.len() - base.len());
            if count == 0 || count > 8 || stack.len() < base.len() + count {
                return None;
            }
            let returned = stack.split_off(stack.len() - count);
            stack.truncate(continuation_depth.unwrap_or(base.len()).max(base.len()));
            if let Some(previous) = expected.take() {
                if previous.len() != returned.len() {
                    return None;
                }
                *expected = Some(
                    previous
                        .into_iter()
                        .zip(returned)
                        .map(|(left, right)| merge_jit_kinds(left, right))
                        .collect::<Option<Vec<_>>>()?,
                );
            } else {
                *expected = Some(returned);
            }
        } else if token == "resultend" {
            let (base, expected) = result_regions.pop()?;
            if !stack.starts_with(&base) {
                return None;
            }
            let returned = stack.split_off(base.len());
            let expected = expected?;
            if returned.len() != expected.len() {
                return None;
            }
            for (expected, returned) in expected.into_iter().zip(returned) {
                stack.push(merge_jit_kinds(expected, returned)?);
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
            branches.push((stack.len(), Some(vec![result]), false));
        } else if token == "if" {
            if stack.pop()? != JitKind::Boolean {
                return None;
            }
            branches.push((stack.len(), None, true));
        } else if token == "ifpresent" {
            let result = *stack.last()?;
            branches.push((stack.len() - 1, Some(vec![result]), true));
        } else if token == "else" {
            let (base, expected, awaits_alternate) = branches.last_mut()?;
            if !*awaits_alternate || stack.len() <= *base {
                return None;
            }
            *expected = Some(stack.split_off(*base));
            *awaits_alternate = false;
        } else if token == "end" {
            let (base, expected, awaits_alternate) = branches.pop()?;
            let expected = expected?;
            if awaits_alternate || stack.len() != base + expected.len() {
                return None;
            }
            let alternate = stack.split_off(base);
            for (expected, alternate) in expected.into_iter().zip(alternate) {
                stack.push(merge_jit_kinds(expected, alternate)?);
            }
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
        } else if token == "dynarraylen" {
            if stack.pop()? != JitKind::Dynamic {
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
        && catches.is_empty()
        && result_regions.is_empty())
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
