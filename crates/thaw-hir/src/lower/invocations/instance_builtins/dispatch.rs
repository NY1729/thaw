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

    /// `buf.readUIntLE(offset, byteLength)` / `buf.writeIntBE(value,
    /// offset, byteLength)` &c. -- the variable-width sibling of
    /// `bytes_numeric_accessor`'s fixed-width names: `byteLength` (1-6,
    /// unlike the fixed accessors' compile-time-known width) is a real
    /// runtime argument, not derivable from the method name alone.
    /// Returns `(signed, big-endian, write)`. `BigUInt64`/`BigInt64` are
    /// a separate, always-8-byte family (`bytes_bigint_accessor`), not
    /// this one -- they need a genuine 64-bit value type (`HirType::
    /// I64`), which this accessor's `f64`-typed value doesn't have.
    fn bytes_variable_width_accessor(property: &str) -> Option<(bool, bool, bool)> {
        let (write, rest) = match property.strip_prefix("write") {
            Some(rest) => (true, rest),
            None => (false, property.strip_prefix("read")?),
        };
        let (signed, rest) = if let Some(rest) = rest.strip_prefix("UInt") {
            (false, rest)
        } else {
            (true, rest.strip_prefix("Int")?)
        };
        let big = match rest {
            "LE" => false,
            "BE" => true,
            _ => return None,
        };
        Some((signed, big, write))
    }

    /// `buf.readBigInt64LE(offset)` / `buf.writeBigUInt64BE(value,
    /// offset)` &c. -- always a full 8 bytes, and the one `Buffer`
    /// accessor family whose value is `HirType::I64` (a genuine 64-bit
    /// integer) rather than `F64` (which can only exactly represent 53
    /// bits) -- see `lower_native_bytes_bigint_accessor`'s doc comment
    /// for the signed/unsigned bit-pattern-sharing note. Returns
    /// `(signed, big-endian, write)`.
    fn bytes_bigint_accessor(property: &str) -> Option<(bool, bool, bool)> {
        let (write, rest) = match property.strip_prefix("write") {
            Some(rest) => (true, rest),
            None => (false, property.strip_prefix("read")?),
        };
        let rest = rest.strip_prefix("Big")?;
        let (signed, rest) = if let Some(rest) = rest.strip_prefix("UInt") {
            (false, rest)
        } else {
            (true, rest.strip_prefix("Int")?)
        };
        let big = match rest {
            "64LE" => false,
            "64BE" => true,
            _ => return None,
        };
        Some((signed, big, write))
    }

    fn is_native_instance_builtin(property: &str) -> bool {
        Self::bytes_numeric_accessor(property).is_some()
            || Self::bytes_variable_width_accessor(property).is_some()
            || Self::bytes_bigint_accessor(property).is_some()
            || matches!(
            property,
            "charAt" | "charCodeAt" | "codePointAt" | "concat" | "trim" | "trimStart" | "trimEnd"
                | "repeat" | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "toExponential"
                | "localeCompare"
                | "normalize" | "split" | "replace" | "replaceAll" | "test" | "match" | "search"
                | "matchAll" | "exec" | "toLowerCase" | "toUpperCase" | "toLocaleLowerCase"
                | "toLocaleUpperCase" | "isWellFormed" | "toWellFormed" | "toReversed"
                | "sort" | "toSorted" | "some" | "every"
                | "find" | "findIndex" | "findLast" | "findLastIndex" | "reduce" | "reduceRight"
                | "toSpliced" | "at" | "with" | "flat" | "flatMap" | "map" | "filter"
                | "forEach" | "slice" | "subarray" | "substring" | "substr" | "hasOwnProperty"
                | "propertyIsEnumerable" | "isPrototypeOf" | "deref" | "group" | "groupToMap"
                | "copyWithin" | "fill" | "reverse" | "join"
                | "push"
                | "pop" | "shift" | "unshift" | "splice" | "indexOf" | "lastIndexOf"
                | "next" | "return" | "throw"
                | "includes" | "startsWith" | "endsWith" | "equals" | "copy" | "toString"
                | "toHex" | "toBase64" | "setFromHex" | "setFromBase64"
                | "valueOf" | "toLocaleString" | "toLocaleDateString" | "toLocaleTimeString"
                | "getTime"
                | "setTime" | "toISOString" | "getFullYear" | "getYear" | "setYear"
                | "getMonth" | "getDate" | "getDay"
                | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds" | "getTimezoneOffset" | "getUTCFullYear"
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
        if let Some(result) = self.lower_json_array_method(member, property, call)? {
            return Ok(result);
        }
        // The private Date field layout is not proof of a native Date.
        // Capture arguments before the internal-slot check, then pass only
        // a verified allocation to every Date getter/setter/converter.
        if matches!(property.sym.as_ref(),
            "getTime" | "setTime" | "getYear" | "setYear" | "toISOString"
            | "getFullYear" | "getMonth" | "getDate" | "getDay"
            | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds"
            | "getTimezoneOffset" | "getUTCFullYear" | "getUTCMonth"
            | "getUTCDate" | "getUTCDay" | "getUTCHours" | "getUTCMinutes"
            | "getUTCSeconds" | "getUTCMilliseconds" | "setFullYear"
            | "setMonth" | "setDate" | "setHours" | "setMinutes"
            | "setSeconds" | "setMilliseconds" | "setUTCFullYear"
            | "setUTCMonth" | "setUTCDate" | "setUTCHours"
            | "setUTCMinutes" | "setUTCSeconds" | "setUTCMilliseconds"
            | "toJSON" | "toDateString" | "toTimeString" | "toUTCString"
            | "toString" | "valueOf" | "toLocaleString"
            | "toLocaleDateString" | "toLocaleTimeString") {
            let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
            // Preserve Bytes here: normalizing it to Array would change
            // the unrelated `Buffer.toString(encoding)` dispatch.
            let receiver_type = self.infer_expr_type_inner(&receiver)?;
            let receiver_name = format!("__thaw_date_method_receiver_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(receiver_name.clone(), receiver_type.clone());
            let mut bindings = vec![(receiver_name.clone(), receiver_type.clone(), receiver)];
            let (arguments, spread_bindings) = self.lower_native_spread_values(
                &call.args, &format!("{}.{}", "native receiver", property.sym),
            )?;
            bindings.extend(spread_bindings);
            let mut synthetic_call = call.clone();
            synthetic_call.args.clear();
            for argument in arguments {
                let ty = self.infer_expr_type(&argument)?;
                let name = format!("__thaw_date_method_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                bindings.push((name.clone(), ty, argument));
                synthetic_call.args.push(swc_ecma_ast::ExprOrSpread {
                    spread: None,
                    expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                        name.into(), member.span,
                    ))),
                });
            }
            let checked_name = format!("__thaw_date_method_checked_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(checked_name.clone(), receiver_type.clone());
            let checked = if receiver_type == date_object_type() {
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_date_assert_native_identity".into())),
                    vec![HirExpr::Var(receiver_name)],
                )
            } else {
                HirExpr::Var(receiver_name)
            };
            bindings.push((checked_name.clone(), receiver_type, checked));
            let synthetic_member = MemberExpr {
                obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    checked_name.into(), member.span,
                ))),
                ..member.clone()
            };
            let result = self.lower_native_instance_builtin_unchecked(
                &synthetic_member, property, &synthetic_call,
            )?;
            return self.wrap_call_argument_bindings(result, &bindings);
        }
        self.lower_native_instance_builtin_unchecked(member, property, call)
    }

    /// An `Array.prototype` method on an `any` (Json) receiver: when the value
    /// is an array at run time, decode it to a native `Array<Json>` copy, run
    /// the ordinary native method on the copy, and write a mutated copy back
    /// into the same Json array (so every alias of it sees the change);
    /// otherwise call the method on the live value like any other `any` method.
    /// `None` when the property or receiver is not covered.
    fn lower_json_array_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let name = property.sym.as_ref();
        let mutates = matches!(
            name,
            "push" | "pop" | "shift" | "unshift" | "splice" | "sort" | "reverse" | "fill" | "copyWithin"
        );
        // These return the receiver array itself, not the copy.
        let returns_receiver = matches!(name, "sort" | "reverse" | "fill" | "copyWithin");
        if !(mutates
            || matches!(
                name,
                "map" | "filter" | "reduce" | "reduceRight" | "some" | "every" | "find" | "findIndex"
                    | "findLast" | "findLastIndex" | "flat" | "flatMap" | "toSorted" | "toReversed"
                    | "toSpliced" | "with" | "forEach" | "slice" | "concat" | "at"
            ))
            || !matches!(member.obj.as_ref(), Expr::Ident(_) | Expr::Member(_))
            || self.peek_type_without_lowering(&member.obj) != Some(HirType::Json)
        {
            return Ok(None);
        }
        let receiver = self.lower_required_member_receiver(&member.obj, name)?;
        if self.infer_expr_type(&receiver)? != HirType::Json {
            return Ok(None);
        }
        let id = self.next_binding;
        self.next_binding += 1;
        let receiver_name = format!("__thaw_json_array_receiver_{id}");
        let copy_name = format!("__thaw_json_array_copy_{id}");
        let result_name = format!("__thaw_json_array_result_{id}");
        let array_type = HirType::Array(Box::new(HirType::Json));
        self.scope.insert(receiver_name.clone(), HirType::Json);
        self.scope.insert(copy_name.clone(), array_type.clone());
        let ident = |name: &str| {
            Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(name.into(), member.span)))
        };
        let mut copy_member = member.clone();
        copy_member.obj = ident(&copy_name);
        let mut copy_call = call.clone();
        copy_call.callee = Callee::Expr(Box::new(Expr::Member(copy_member.clone())));
        let native = self.lower_native_instance_builtin_unchecked(&copy_member, property, &copy_call)?;
        let native_type = self.infer_expr_type(&native)?;
        let mut bindings = vec![(
            copy_name.clone(),
            array_type.clone(),
            self.coerce_to_declared(&array_type, HirExpr::Var(receiver_name.clone()))?,
        )];
        let void = native_type == HirType::Void;
        let undefined = HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_undefined".into())), Vec::new());
        let mut tail = HirExpr::Var(receiver_name.clone());
        if !returns_receiver {
            if void {
                tail = HirExpr::EvalThen(Box::new(native.clone()), Box::new(undefined.clone()));
            } else {
                self.scope.insert(result_name.clone(), native_type.clone());
                bindings.push((result_name.clone(), native_type.clone(), native));
                tail = self.coerce_to_declared(&HirType::Json, HirExpr::Var(result_name.clone()))?;
            }
        } else {
            self.scope.insert(result_name.clone(), native_type.clone());
            bindings.push((result_name.clone(), native_type, native));
        }
        if mutates {
            let written = format!("__thaw_json_array_written_{id}");
            self.scope.insert(written.clone(), HirType::Json);
            let copy_json = self.coerce_to_declared(&HirType::Json, HirExpr::Var(copy_name.clone()))?;
            bindings.push((
                written,
                HirType::Json,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_array_assign".into())),
                    vec![HirExpr::Var(receiver_name.clone()), copy_json],
                ),
            ));
        }
        let array_branch = self.wrap_call_argument_bindings(tail, &bindings)?;
        // Names that are also string/Map/Set methods keep their existing
        // lowering for a non-array value; the rest use a live dynamic call.
        // A Map/Set `forEach` callback is lowered for Json parameters, so an
        // annotated callback takes the live dynamic call instead.
        let loose = |pat: &swc_ecma_ast::Pat| match pat {
            swc_ecma_ast::Pat::Ident(ident) => ident.type_ann.as_ref().is_none_or(|ann| {
                matches!(ann.type_ann.as_ref(), swc_ecma_ast::TsType::TsKeywordType(keyword)
                    if keyword.kind == swc_ecma_ast::TsKeywordTypeKind::TsAnyKeyword)
            }),
            _ => false,
        };
        let annotated_callback = call.args.first().is_some_and(|argument| match argument.expr.as_ref() {
            Expr::Arrow(arrow) => !arrow.params.iter().all(loose),
            Expr::Fn(function) => !function.function.params.iter().all(|param| loose(&param.pat)),
            _ => true,
        });
        let non_array = if matches!(name, "slice" | "concat" | "at") || (name == "forEach" && !annotated_callback) {
            self.lower_native_instance_builtin_unchecked(member, property, call).ok()
        } else {
            None
        };
        let non_array = match non_array {
            Some(expr) => expr,
            None => self.lower_dynamic_value_method_call(
                &Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(receiver_name.clone().into(), member.span)),
                name,
                &call.args,
                None,
            )?,
        };
        let non_array = if self.infer_expr_type(&non_array)? == HirType::Void {
            HirExpr::EvalThen(Box::new(non_array), Box::new(undefined))
        } else {
            self.coerce_to_declared(&HirType::Json, non_array)?
        };
        let (array_branch, dynamic, ty) = (array_branch, non_array, HirType::Json);
        // A host-owned array (a live view of a native array, or a JS array) is mutated in place
        // by the live dynamic call; only a plain JSON array takes the copy-and-write-back path.
        let is_array = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_is_array".into())),
            vec![HirExpr::Var(receiver_name.clone())],
        );
        let has_handle = Self::strict_has_handle(Self::strict_json_handle(HirExpr::Var(receiver_name.clone())));
        let condition = Self::strict_if(
            is_array,
            Self::strict_if(has_handle, HirExpr::Lit(HirLit::Bool(false)), HirExpr::Lit(HirLit::Bool(true))),
            HirExpr::Lit(HirLit::Bool(false)),
        );
        let result = HirExpr::Conditional(Box::new(condition), Box::new(array_branch), Box::new(dynamic), ty);
        Ok(Some(self.wrap_call_argument_bindings(result, &[(receiver_name, HirType::Json, receiver)])?))
    }

    fn lower_native_instance_builtin_unchecked(
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
            | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "toExponential"
                            | "toLowerCase" | "toUpperCase" | "toLocaleLowerCase" | "toLocaleUpperCase"
                | "isWellFormed" | "toWellFormed" => {
                self.lower_native_scalar_method(member, property, call)
            }
            "toReversed" | "sort" | "toSorted" | "some" | "every" | "find" | "findIndex"
            | "findLast" | "findLastIndex" | "reduce" | "reduceRight" | "toSpliced" | "at"
            | "with" | "flat" | "flatMap" | "map" | "filter" => {
                self.lower_native_array_transform_method(member, property, call)
            }
            "forEach" | "slice" | "subarray" | "substring" | "substr" | "copyWithin" | "fill"
            | "reverse" | "join" | "push" | "pop" | "shift" | "unshift" | "splice" | "indexOf"
            | "lastIndexOf" | "includes" | "startsWith" | "endsWith" | "next" | "return"
            | "throw" => self.lower_native_array_mutation_method(member, property, call),
            "hasOwnProperty" | "propertyIsEnumerable" => {
                self.lower_native_has_own_property(member, call)
            }
            "isPrototypeOf" => {
                if call.args.len() != 1 {
                    return Err("native `.isPrototypeOf()` expects exactly one argument".into());
                }
                Err("native `.isPrototypeOf()` is unavailable without a prototype chain".into())
            }
            "deref" => self.lower_native_weakref_deref(member, call),
            "group" | "groupToMap" => self.lower_native_array_group(member, property, call),
            "getTime" | "setTime" | "toISOString" | "getFullYear" | "getYear" | "setYear"
            | "getMonth" | "getDate"
            | "getDay" | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds" | "getTimezoneOffset"
            | "getUTCFullYear" | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours"
            | "getUTCMinutes" | "getUTCSeconds" | "getUTCMilliseconds" | "setFullYear"
            | "setMonth" | "setDate" | "setHours" | "setMinutes" | "setSeconds"
            | "setMilliseconds" | "setUTCFullYear" | "setUTCMonth" | "setUTCDate"
            | "setUTCHours" | "setUTCMinutes" | "setUTCSeconds" | "setUTCMilliseconds" => {
                self.lower_native_date_method(member, property, call)
            }
            "get" | "set" | "add" | "has" | "delete" | "clear" | "getOrInsert"
            | "getOrInsertComputed" | "union" | "intersection"
            | "difference" | "symmetricDifference" | "isSubsetOf" | "isSupersetOf"
            | "isDisjointFrom" | "keys" | "values" | "entries" => {
                self.lower_native_map_set_method(member, property, call)
            }
            "toJSON" | "toDateString" | "toTimeString" | "toUTCString" | "toString"
            | "valueOf" | "equals" | "toLocaleString" | "toLocaleDateString"
            | "toLocaleTimeString" => {
                self.lower_native_conversion_method(member, property, call)
            }
            "copy" => self.lower_native_bytes_copy(member, call),
            "toHex" | "toBase64" | "setFromHex" | "setFromBase64" => {
                self.lower_native_bytes_encoding_method(member, property, call)
            }
            other if Self::bytes_numeric_accessor(other).is_some() => {
                self.lower_native_bytes_accessor(member, property, call)
            }
            other if Self::bytes_variable_width_accessor(other).is_some() => {
                self.lower_native_bytes_variable_width_accessor(member, property, call)
            }
            other if Self::bytes_bigint_accessor(other).is_some() => {
                self.lower_native_bytes_bigint_accessor(member, property, call)
            }
            _ => unreachable!("native instance builtin dispatch was checked before lowering"),
        }
    }

    /// `WeakRef<T>.deref()`: the constructor is modeled as a one-element
    /// array holding a *strong* reference (see `lower/expressions/lowering.rs`),
    /// so `deref()` is an index-0 read that always yields the target.
    fn lower_native_weakref_deref(
        &mut self,
        member: &MemberExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        if !call.args.is_empty() {
            return Err("native `.deref()` expects no arguments".into());
        }
        let receiver = self.lower_required_member_receiver(&member.obj, "deref")?;
        let receiver_type = self.infer_expr_type(&receiver)?;
        let element = match &receiver_type {
            HirType::Array(element) => element.as_ref().clone(),
            HirType::Tuple(elements) => elements
                .first()
                .cloned()
                .ok_or("native `.deref()` expects a non-empty `WeakRef`")?,
            _ => return Err("native `.deref()` expects a `WeakRef`".into()),
        };
        Ok(HirExpr::TypedIndex(
            Box::new(receiver),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            element,
        ))
    }
}
