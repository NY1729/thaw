impl<'a> FnLowerer<'a> {
    /// A fixed-width `Buffer` numeric accessor -- `readUInt16BE`,
    /// `writeInt32LE`, `readDoubleLE`, &c. Returns `(width bytes, signed,
    /// float, big-endian, write)`. These names are `Buffer`-idiomatic and
    /// don't collide with any array/string/Date/Map method, so an
    /// unconditional match here is safe; `lower_native_bytes_accessor`
    /// still rejects a non-`Bytes` receiver.
    fn bytes_numeric_accessor(property: &str) -> Option<(usize, bool, bool, bool, bool)> {
        let (write, rest) = match property.strip_prefix("write") {
            Some(rest) => (true, rest),
            None => (false, property.strip_prefix("read")?),
        };
        if let Some(endian) = rest.strip_prefix("Float") {
            let big = match endian {
                "LE" => false,
                "BE" => true,
                _ => return None,
            };
            return Some((4, true, true, big, write));
        }
        if let Some(endian) = rest.strip_prefix("Double") {
            let big = match endian {
                "LE" => false,
                "BE" => true,
                _ => return None,
            };
            return Some((8, true, true, big, write));
        }
        let (signed, rest) = if let Some(rest) = rest.strip_prefix("UInt") {
            (false, rest)
        } else {
            (true, rest.strip_prefix("Int")?)
        };
        let (width, big) = match rest {
            "8" => (1, false),
            "16LE" => (2, false),
            "16BE" => (2, true),
            "32LE" => (4, false),
            "32BE" => (4, true),
            _ => return None,
        };
        Some((width, signed, false, big, write))
    }

    fn is_native_instance_builtin(property: &str) -> bool {
        Self::bytes_numeric_accessor(property).is_some()
            || matches!(
            property,
            "charAt" | "charCodeAt" | "codePointAt" | "concat" | "trim" | "trimStart" | "trimEnd"
                | "repeat" | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "localeCompare"
                | "normalize" | "split" | "replace" | "replaceAll" | "test" | "match" | "search"
                | "matchAll" | "exec" | "toLowerCase" | "toUpperCase" | "isWellFormed"
                | "toWellFormed" | "toReversed" | "sort" | "toSorted" | "some" | "every"
                | "find" | "findIndex" | "findLast" | "findLastIndex" | "reduce" | "reduceRight"
                | "toSpliced" | "at" | "with" | "flat" | "flatMap" | "map" | "filter"
                | "forEach" | "slice" | "subarray" | "copyWithin" | "fill" | "reverse" | "join"
                | "push"
                | "pop" | "shift" | "unshift" | "splice" | "indexOf" | "lastIndexOf"
                | "next" | "return" | "throw"
                | "includes" | "startsWith" | "endsWith" | "equals" | "toString" | "valueOf"
                | "getTime"
                | "setTime" | "toISOString" | "getFullYear" | "getMonth" | "getDate" | "getDay"
                | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds" | "getUTCFullYear"
                | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours" | "getUTCMinutes"
                | "getUTCSeconds" | "getUTCMilliseconds" | "setFullYear" | "setMonth" | "setDate"
                | "setHours" | "setMinutes" | "setSeconds" | "setMilliseconds" | "setUTCFullYear"
                | "setUTCMonth" | "setUTCDate" | "setUTCHours" | "setUTCMinutes" | "setUTCSeconds"
                | "setUTCMilliseconds" | "toDateString" | "toTimeString" | "toUTCString" | "toJSON"
                | "keys" | "values" | "entries" | "union" | "intersection" | "difference"
                | "symmetricDifference"
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
            "forEach" | "slice" | "subarray" | "copyWithin" | "fill" | "reverse" | "join" | "push"
            | "pop" | "shift" | "unshift" | "splice" | "indexOf" | "lastIndexOf"
            | "includes" | "startsWith" | "endsWith" | "next" | "return" | "throw" => {
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
            | "valueOf" | "equals" => self.lower_native_conversion_method(member, property, call),
            other if Self::bytes_numeric_accessor(other).is_some() => {
                self.lower_native_bytes_accessor(member, property, call)
            }
            _ => unreachable!("native instance builtin dispatch was checked before lowering"),
        }
    }
}
