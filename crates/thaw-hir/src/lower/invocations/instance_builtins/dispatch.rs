impl<'a> FnLowerer<'a> {
    fn is_native_instance_builtin(property: &str) -> bool {
        matches!(
            property,
            "charAt" | "charCodeAt" | "codePointAt" | "concat" | "trim" | "trimStart" | "trimEnd"
                | "repeat" | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "localeCompare"
                | "normalize" | "split" | "replace" | "replaceAll" | "test" | "match" | "search"
                | "matchAll" | "exec" | "toLowerCase" | "toUpperCase" | "isWellFormed"
                | "toWellFormed" | "toReversed" | "sort" | "toSorted" | "some" | "every"
                | "find" | "findIndex" | "findLast" | "findLastIndex" | "reduce" | "reduceRight"
                | "toSpliced" | "at" | "with" | "flat" | "flatMap" | "map" | "filter"
                | "forEach" | "slice" | "copyWithin" | "fill" | "reverse" | "join" | "push"
                | "pop" | "shift" | "unshift" | "splice" | "indexOf" | "lastIndexOf"
                | "includes" | "startsWith" | "endsWith" | "toString" | "valueOf" | "getTime"
                | "setTime" | "toISOString" | "getFullYear" | "getMonth" | "getDate" | "getDay"
                | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds" | "getUTCFullYear"
                | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours" | "getUTCMinutes"
                | "getUTCSeconds" | "getUTCMilliseconds" | "setFullYear" | "setMonth" | "setDate"
                | "setHours" | "setMinutes" | "setSeconds" | "setMilliseconds" | "setUTCFullYear"
                | "setUTCMonth" | "setUTCDate" | "setUTCHours" | "setUTCMinutes" | "setUTCSeconds"
                | "setUTCMilliseconds" | "toDateString" | "toTimeString" | "toUTCString" | "toJSON"
                | "keys" | "values" | "entries" | "get" | "set" | "add" | "has" | "clear"
                | "delete" | "union" | "intersection" | "difference" | "symmetricDifference"
                | "isSubsetOf" | "isSupersetOf" | "isDisjointFrom"
        )
    }

    fn lower_native_instance_builtin(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        match property.sym.as_ref() {
            "charAt" | "charCodeAt" | "localeCompare" | "normalize" | "split" => {
                self.lower_native_text_method(member, property, call)
            }
            "replace" | "replaceAll" | "test" | "exec" | "match" | "matchAll" | "search" => {
                self.lower_native_regex_method(member, property, call)
            }
            "codePointAt" | "concat" | "trim" | "trimStart" | "trimEnd" | "repeat"
            | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "toLowerCase"
            | "toUpperCase" | "isWellFormed" | "toWellFormed" => {
                self.lower_native_scalar_method(member, property, call)
            }
            "toReversed" | "sort" | "toSorted" | "some" | "every" | "find" | "findIndex"
            | "findLast" | "findLastIndex" | "reduce" | "reduceRight" | "toSpliced" | "at"
            | "with" | "flat" | "flatMap" | "map" | "filter" => {
                self.lower_native_array_transform_method(member, property, call)
            }
            "forEach" | "slice" | "copyWithin" | "fill" | "reverse" | "join" | "push"
            | "pop" | "shift" | "unshift" | "splice" | "indexOf" | "lastIndexOf"
            | "includes" | "startsWith" | "endsWith" => {
                self.lower_native_array_mutation_method(member, property, call)
            }
            "getTime" | "setTime" | "toISOString" | "getFullYear" | "getMonth" | "getDate"
            | "getDay" | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds"
            | "getUTCFullYear" | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours"
            | "getUTCMinutes" | "getUTCSeconds" | "getUTCMilliseconds" | "setFullYear"
            | "setMonth" | "setDate" | "setHours" | "setMinutes" | "setSeconds"
            | "setMilliseconds" | "setUTCFullYear" | "setUTCMonth" | "setUTCDate"
            | "setUTCHours" | "setUTCMinutes" | "setUTCSeconds" | "setUTCMilliseconds" => {
                self.lower_native_date_method(member, property, call)
            }
            "get" | "set" | "add" | "has" | "delete" | "clear" | "union" | "intersection"
            | "difference" | "symmetricDifference" | "isSubsetOf" | "isSupersetOf"
            | "isDisjointFrom" | "keys" | "values" | "entries" => {
                self.lower_native_map_set_method(member, property, call)
            }
            "toJSON" | "toDateString" | "toTimeString" | "toUTCString" | "toString"
            | "valueOf" => self.lower_native_conversion_method(member, property, call),
            _ => unreachable!("native instance builtin dispatch was checked before lowering"),
        }
    }
}
