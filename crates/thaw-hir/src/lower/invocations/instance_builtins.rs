fn regex_object_type() -> HirType {
    HirType::Object(vec![
        ("source".to_string(), HirType::Str),
        ("flags".to_string(), HirType::Str),
    ])
}

/// `Date` is a fixed native object with a single millisecond-since-epoch
/// `timestamp` field (reusing the existing object machinery, like
/// `regex_object_type`, rather than a new value representation).
/// `thaw-runtime`'s calendar math is UTC-only -- there is no host timezone
/// database, so "local" `Date` methods alias their UTC counterparts.
fn date_object_type() -> HirType {
    HirType::Object(vec![("timestamp".to_string(), HirType::F64)])
}

/// `Map`/`Set`'s key/element type selects which family of native
/// `__thaw_map_*` intrinsics to call -- `"num"` or `"str"`. Any other key
/// type is rejected at `Map<K, V>`/`Set<T>` resolution time
/// (`type_resolution.rs`) and at `new Map<K, V>()`/`new Set<T>()`
/// construction time, so this should never actually fail once a `Map`/
/// `Set` value exists, but a lowering bug elsewhere producing one anyway
/// shouldn't panic.
fn map_key_intrinsic_suffix(key_type: &HirType) -> Result<&'static str, String> {
    match key_type {
        HirType::F64 => Ok("num"),
        HirType::Str => Ok("str"),
        other => Err(format!(
            "Map/Set keys must be `number` or `string`, got {other:?}"
        )),
    }
}

/// Chooses which `__thaw_map_{num,str}_get_*` variant decodes a `Map`
/// value of `value_type` correctly, and whether the raw call result needs
/// wrapping in `HirExpr::TypedClosure` to recover a pointer-shaped type
/// the intrinsic name alone can't carry (unlike `F64`/`Bool`, where the
/// result type is always the same regardless of context).
fn map_value_get_suffix(value_type: &HirType) -> Result<(&'static str, bool), String> {
    match value_type {
        HirType::F64 => Ok(("f64", false)),
        HirType::Bool | HirType::Undefined | HirType::Null => Ok(("bool", false)),
        HirType::I64 | HirType::JsValue => Err(format!(
            "Map/Set values of type {value_type:?} are not supported"
        )),
        _ => Ok(("ptr", true)),
    }
}

impl<'a> FnLowerer<'a> {
    /// `get`/`set`/`has`/`delete`/`add`/`clear` are common enough method
    /// names that a user's own class/object could plausibly define them
    /// too (unlike e.g. `charCodeAt`), so unlike every other native
    /// instance builtin, these can't be claimed by property name alone --
    /// `is_native_instance_builtin` doesn't see the receiver. This reads
    /// the receiver's type via already-established scope/interface data
    /// with no lowering (and so no risk of double-evaluating a
    /// side-effecting receiver expression like a function call), so a
    /// `Map`/`Set` variable, `this.field`, or a nested `a.b.c` member
    /// chain is recognized; anything else (for example a receiver that is
    /// itself a call, like `getMap().get(x)`) safely falls through to
    /// ordinary property/method-call handling instead.
    fn peek_type_without_lowering(&self, expr: &Expr) -> Option<HirType> {
        match expr {
            Expr::Ident(ident) => {
                let resolved = self.resolve_binding(ident.sym.as_ref());
                self.scope.get(&resolved).cloned()
            }
            Expr::This(_) => {
                let resolved = self.resolve_binding("this");
                self.scope.get(&resolved).cloned()
            }
            Expr::Member(member) => {
                let MemberProp::Ident(field) = &member.prop else {
                    return None;
                };
                let HirType::Object(fields) = self.peek_type_without_lowering(&member.obj)?
                else {
                    return None;
                };
                fields
                    .into_iter()
                    .find(|(name, _)| name == field.sym.as_ref())
                    .map(|(_, ty)| ty)
            }
            // `map.set(k, v).set(k2, v2)`/`set.add(a).add(b)`: `set`/`add`
            // return the receiver itself for chaining, so a call to either
            // one has the same type as ITS OWN receiver -- recurse without
            // looking at the call's arguments (irrelevant to the type, and
            // this must stay lowering-free).
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let Expr::Member(member) = callee.as_ref() else {
                    return None;
                };
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                if !matches!(property.sym.as_ref(), "set" | "add") {
                    return None;
                }
                let receiver_type = self.peek_type_without_lowering(&member.obj)?;
                matches!(receiver_type, HirType::Map(_, _) | HirType::Set(_))
                    .then_some(receiver_type)
            }
            _ => None,
        }
    }

    fn receiver_is_map_or_set(&self, expr: &Expr) -> bool {
        matches!(
            self.peek_type_without_lowering(expr),
            Some(HirType::Map(_, _) | HirType::Set(_))
        )
    }

    fn is_native_instance_builtin(property: &str) -> bool {
        matches!(
            property,
            "charCodeAt" | "codePointAt" | "concat" | "trim" | "trimStart" | "trimEnd"
                | "repeat" | "padStart" | "padEnd" | "toFixed" | "toPrecision" | "localeCompare"
                | "normalize" | "split" | "replace" | "replaceAll" | "test" | "match" | "search"
                | "exec"
                | "toLowerCase" | "toUpperCase" | "isWellFormed" | "toWellFormed"
                | "toReversed" | "sort" | "toSorted" | "some" | "every" | "find"
                | "findIndex" | "findLast" | "findLastIndex" | "reduce" | "reduceRight"
                | "toSpliced" | "at" | "with" | "flat" | "flatMap" | "map" | "filter"
                | "forEach" | "slice" | "copyWithin" | "fill" | "reverse" | "join"
                | "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith"
                | "toString" | "valueOf"
                | "getTime" | "setTime" | "toISOString"
                | "getFullYear" | "getMonth" | "getDate" | "getDay" | "getHours"
                | "getMinutes" | "getSeconds" | "getMilliseconds"
                | "getUTCFullYear" | "getUTCMonth" | "getUTCDate" | "getUTCDay"
                | "getUTCHours" | "getUTCMinutes" | "getUTCSeconds" | "getUTCMilliseconds"
                | "setFullYear" | "setMonth" | "setDate" | "setHours" | "setMinutes"
                | "setSeconds" | "setMilliseconds"
                | "setUTCFullYear" | "setUTCMonth" | "setUTCDate" | "setUTCHours"
                | "setUTCMinutes" | "setUTCSeconds" | "setUTCMilliseconds"
                | "toDateString" | "toTimeString" | "toUTCString" | "toJSON"
        )
    }

    fn lower_native_instance_builtin(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
                if property.sym == *"charCodeAt" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "charCodeAt receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.charCodeAt")?;
                    if arguments.len() > 1 {
                        return Err("native `.charCodeAt()` expects zero or one argument".into());
                    }
                    let index = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_char_code_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_char_code_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_char_code_at".to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(index_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((index_name, HirType::F64, index));
                    return self.wrap_call_argument_bindings(
                        result,
                        &bindings,
                    );
                }
                if property.sym == *"localeCompare" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "localeCompare receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.localeCompare")?;
                    let [other] = arguments.as_slice() else {
                        return Err("native `.localeCompare()` expects exactly one argument".into());
                    };
                    let other = self.coerce_primitive_to_string(other.clone())?;
                    let receiver_name =
                        format!("__thaw_locale_compare_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let other_name = format!("__thaw_locale_compare_other_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(other_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_locale_compare".to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(other_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((other_name, HirType::Str, other));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"normalize" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "normalize receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.normalize")?;
                    if arguments.len() > 1 {
                        return Err("native `.normalize()` expects zero or one argument".into());
                    }
                    let form = match arguments.first() {
                        Some(argument) => self.coerce_primitive_to_string(argument.clone())?,
                        None => HirExpr::Lit(HirLit::Str("NFC".into())),
                    };
                    let receiver_name =
                        format!("__thaw_normalize_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let form_name = format!("__thaw_normalize_form_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_normalize_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(form_name.clone(), HirType::Str);
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_normalize".into())),
                                vec![var(&receiver_name), var(&form_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "The normalization form should be one of NFC, NFD, NFKC, NFKD."
                                    .into(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(var(&raw_name))),
                    ]);
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            HirType::Str,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((form_name, HirType::Str, form));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"split" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "split receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.split")?;
                    if arguments.len() > 2 {
                        return Err(
                            "native `.split()` expects zero, one or two arguments".into()
                        );
                    }
                    if arguments.is_empty() {
                        let receiver_name =
                            format!("__thaw_split_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayLit(vec![HirExpr::Var(receiver_name.clone())]),
                            &[(receiver_name, HirType::Str, receiver)],
                        );
                    }
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(&arguments[0])? == regex_type {
                        if arguments.len() > 1 {
                            return Err(
                                "native `.split()` does not support a limit with a RegExp separator"
                                    .into(),
                            );
                        }
                        let pattern = arguments[0].clone();
                        let receiver_name =
                            format!("__thaw_regex_split_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let pattern_name =
                            format!("__thaw_regex_split_pattern_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(pattern_name.clone(), regex_type.clone());
                        let source = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "source".to_string(),
                        );
                        let flags = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "flags".to_string(),
                        );
                        let array_type = HirType::Array(Box::new(HirType::Str));
                        let raw_name = format!("__thaw_regex_split_raw_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(raw_name.clone(), array_type.clone());
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(
                                raw_name.clone(),
                                array_type.clone(),
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_regex_split".to_string())),
                                    vec![HirExpr::Var(receiver_name.clone()), source, flags],
                                ),
                            ),
                            HirStmt::If(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                    vec![HirExpr::Var(raw_name.clone())],
                                ),
                                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                    "invalid regular expression".into(),
                                )))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::Var(raw_name))),
                        ]);
                        let mut referenced = BTreeSet::new();
                        collect_referenced_bindings(&body, &mut referenced);
                        let captures = referenced
                            .into_iter()
                            .filter_map(|captured| {
                                self.scope
                                    .get(&captured)
                                    .cloned()
                                    .map(|ty| HirParam { name: captured, ty })
                            })
                            .collect();
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                array_type,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((pattern_name, regex_type, pattern));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let separator = self.coerce_primitive_to_string(arguments[0].clone())?;
                    let limit = match arguments.get(1) {
                        Some(argument) => self.coerce_primitive_to_number(argument.clone())?,
                        None => HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                    };
                    let receiver_name = format!("__thaw_split_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let separator_name =
                        format!("__thaw_split_separator_{}", self.next_binding);
                    self.next_binding += 1;
                    let limit_name = format!("__thaw_split_limit_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(separator_name.clone(), HirType::Str);
                    self.scope.insert(limit_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_split".into())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(separator_name.clone()),
                            HirExpr::Var(limit_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((separator_name, HirType::Str, separator));
                    bindings.push((limit_name, HirType::F64, limit));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "replace" | "replaceAll") {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "replace receiver")?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("String.{}", property.sym),
                    )?;
                    let [search, replacement] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly two arguments",
                            property.sym
                        ));
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(search)? == regex_type {
                        let pattern = search.clone();
                        let replacement = self.coerce_primitive_to_string(replacement.clone())?;
                        let intrinsic = if property.sym == *"replace" {
                            "__thaw_regex_replace"
                        } else {
                            "__thaw_regex_replace_all"
                        };
                        let receiver_name =
                            format!("__thaw_regex_replace_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let pattern_name =
                            format!("__thaw_regex_replace_pattern_{}", self.next_binding);
                        self.next_binding += 1;
                        let replacement_name =
                            format!("__thaw_regex_replace_value_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(pattern_name.clone(), regex_type.clone());
                        self.scope.insert(replacement_name.clone(), HirType::Str);
                        let source = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "source".to_string(),
                        );
                        let flags = HirExpr::PropAccess(
                            Box::new(HirExpr::Var(pattern_name.clone())),
                            regex_type.clone(),
                            "flags".to_string(),
                        );
                        let raw_name = format!("__thaw_regex_replace_raw_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(raw_name.clone(), HirType::Str);
                        let error_message = if property.sym == *"replace" {
                            "invalid regular expression"
                        } else {
                            "replaceAll must be called with a global RegExp"
                        };
                        let body = HirExpr::Block(vec![
                            HirStmt::Let(
                                raw_name.clone(),
                                HirType::Str,
                                HirExpr::Call(
                                    Box::new(HirExpr::Var(intrinsic.to_string())),
                                    vec![
                                        HirExpr::Var(receiver_name.clone()),
                                        source,
                                        flags,
                                        HirExpr::Var(replacement_name.clone()),
                                    ],
                                ),
                            ),
                            HirStmt::If(
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                    vec![HirExpr::Var(raw_name.clone())],
                                ),
                                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                    error_message.into(),
                                )))],
                                Vec::new(),
                            ),
                            HirStmt::Return(Some(HirExpr::Var(raw_name))),
                        ]);
                        let mut referenced = BTreeSet::new();
                        collect_referenced_bindings(&body, &mut referenced);
                        let captures = referenced
                            .into_iter()
                            .filter_map(|captured| {
                                self.scope
                                    .get(&captured)
                                    .cloned()
                                    .map(|ty| HirParam { name: captured, ty })
                            })
                            .collect();
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                HirType::Str,
                                Box::new(body),
                            )),
                            Vec::new(),
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((pattern_name, regex_type, pattern));
                        bindings.push((replacement_name, HirType::Str, replacement));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let search = self.coerce_primitive_to_string(search.clone())?;
                    let replacement = self.coerce_primitive_to_string(replacement.clone())?;
                    let intrinsic = if property.sym == *"replace" {
                        "__thaw_string_replace"
                    } else {
                        "__thaw_string_replace_all"
                    };
                    let receiver_name = format!("__thaw_replace_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let search_name = format!("__thaw_replace_search_{}", self.next_binding);
                    self.next_binding += 1;
                    let replacement_name =
                        format!("__thaw_replace_replacement_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(search_name.clone(), HirType::Str);
                    self.scope.insert(replacement_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(search_name.clone()),
                            HirExpr::Var(replacement_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((search_name, HirType::Str, search));
                    bindings.push((replacement_name, HirType::Str, replacement));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"test" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let regex_type = regex_object_type();
                    self.expect_type(&regex_type, &receiver, "RegExp.test receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "RegExp.test")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.test()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_string(value.clone())?;
                    let receiver_name = format!("__thaw_regex_test_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_regex_test_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), regex_type.clone());
                    self.scope.insert(value_name.clone(), HirType::Str);
                    let source = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_regex_test".to_string())),
                        vec![source, flags, HirExpr::Var(value_name.clone())],
                    );
                    let mut bindings = vec![(receiver_name, regex_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::Str, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"exec" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let regex_type = regex_object_type();
                    self.expect_type(&regex_type, &receiver, "RegExp.exec receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "RegExp.exec")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.exec()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_string(value.clone())?;
                    let receiver_name = format!("__thaw_regex_exec_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_regex_exec_value_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_regex_exec_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), regex_type.clone());
                    self.scope.insert(value_name.clone(), HirType::Str);
                    let array_type = HirType::Array(Box::new(HirType::Str));
                    self.scope.insert(raw_name.clone(), array_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_exec".into())),
                                vec![source, flags, var(&value_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                array_type.clone(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&raw_name)),
                            array_type.clone(),
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(array_type));
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, regex_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::Str, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"match" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "match receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.match")?;
                    let [pattern] = arguments.as_slice() else {
                        return Err("native `.match()` expects exactly one argument".into());
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(pattern)? != regex_type {
                        return Err("native `.match()` requires a RegExp argument".into());
                    }
                    let pattern = pattern.clone();
                    let receiver_name = format!("__thaw_match_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let pattern_name = format!("__thaw_match_pattern_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_match_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(pattern_name.clone(), regex_type.clone());
                    let array_type = HirType::Array(Box::new(HirType::Str));
                    self.scope.insert(raw_name.clone(), array_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let source = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(var(&pattern_name)),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            array_type.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_regex_match".into())),
                                vec![var(&receiver_name), source, flags],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                array_type.clone(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&raw_name)),
                            array_type.clone(),
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(array_type));
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((pattern_name, regex_type, pattern));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"search" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "search receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.search")?;
                    let [pattern] = arguments.as_slice() else {
                        return Err("native `.search()` expects exactly one argument".into());
                    };
                    let regex_type = regex_object_type();
                    if self.infer_expr_type(pattern)? != regex_type {
                        return Err("native `.search()` requires a RegExp argument".into());
                    }
                    let pattern = pattern.clone();
                    let receiver_name = format!("__thaw_search_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let pattern_name = format!("__thaw_search_pattern_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(pattern_name.clone(), regex_type.clone());
                    let source = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(pattern_name.clone())),
                        regex_type.clone(),
                        "source".to_string(),
                    );
                    let flags = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(pattern_name.clone())),
                        regex_type.clone(),
                        "flags".to_string(),
                    );
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_regex_search".to_string())),
                        vec![HirExpr::Var(receiver_name.clone()), source, flags],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((pattern_name, regex_type, pattern));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"codePointAt" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "codePointAt receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.codePointAt")?;
                    if arguments.len() > 1 {
                        return Err("native `.codePointAt()` expects zero or one argument".into());
                    }
                    let index = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name =
                        format!("__thaw_code_point_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_code_point_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_code_point_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_code_point_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    self.scope.insert(raw_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |name: String, value| {
                        HirStmt::Expr(HirExpr::Assign(name, Box::new(value)))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&index_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(normalized_name.clone(), number(0.0))],
                        ),
                        assign(
                            normalized_name.clone(),
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                                vec![var(&normalized_name)],
                            ),
                        ),
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::F64,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_code_point_at".into())),
                                vec![var(&receiver_name), var(&normalized_name)],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&raw_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(HirType::F64)))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(var(&raw_name)),
                            HirType::F64,
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(HirType::F64));
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((index_name, HirType::F64, index));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"concat" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "native concat")?;
                    if let HirType::Array(element) = receiver_type {
                        let element = element.as_ref().clone();
                        let array_type = HirType::Array(Box::new(element.clone()));
                        let receiver_name = format!("__thaw_concat_part_0_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(receiver_name.clone(), array_type.clone());
                        let mut bindings = vec![(receiver_name.clone(), array_type, receiver)];
                        bindings.extend(spread_bindings);
                        let mut ordered = vec![HirExpr::Var(receiver_name)];
                        for (position, value) in arguments.into_iter().enumerate() {
                            let actual = self.infer_expr_type(&value)?;
                            let part = if actual == HirType::Array(Box::new(element.clone())) {
                                value
                            } else if actual == element {
                                HirExpr::ArrayLit(vec![value])
                            } else {
                                return Err(format!(
                                    "array concat argument has type {actual:?}, expected {element:?} or an array of it"
                                ));
                            };
                            let ty = self.infer_expr_type(&part)?;
                            let name = format!(
                                "__thaw_concat_part_{}_{}",
                                position + 1,
                                self.next_binding
                            );
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            bindings.push((name.clone(), ty, part));
                            ordered.push(HirExpr::Var(name));
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayConcat(ordered, element),
                            &bindings,
                        );
                    }
                    self.expect_type(&HirType::Str, &receiver, "string concat receiver")?;
                    let receiver_name =
                        format!("__thaw_string_concat_part_0_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    let mut bindings =
                        vec![(receiver_name.clone(), HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    let mut values = vec![HirExpr::Var(receiver_name)];
                    for (position, source) in arguments.into_iter().enumerate() {
                        let source = self.coerce_primitive_to_string(source)?;
                        let name = format!(
                            "__thaw_string_concat_part_{}_{}",
                            position + 1,
                            self.next_binding
                        );
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        bindings.push((name.clone(), HirType::Str, source));
                        values.push(HirExpr::Var(name));
                    }
                    let mut values = values.into_iter();
                    let mut result = values.next().expect("concat always has a receiver");
                    for value in values {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![result, value],
                        );
                    }
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "trim" | "trimStart" | "trimEnd") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string trim receiver")?;
                    let suffix = match property.sym.as_ref() {
                        "trim" => "trim",
                        "trimStart" => "trim_start",
                        "trimEnd" => "trim_end",
                        _ => unreachable!(),
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if property.sym == *"repeat" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string repeat receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.repeat")?;
                    let [count] = arguments.as_slice() else {
                        return Err("native `.repeat()` expects exactly one count".into());
                    };
                    let count = self.coerce_primitive_to_number(count.clone())?;
                    let receiver_name = format!("__thaw_repeat_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let count_name = format!("__thaw_repeat_count_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name = format!("__thaw_repeat_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(count_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "Invalid count value for String.prototype.repeat".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&count_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(number(f64::INFINITY)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_repeat".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((count_name, HirType::F64, count));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if matches!(property.sym.as_ref(), "padStart" | "padEnd") {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string pad receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "String.padStart/padEnd")?;
                    if arguments.is_empty() || arguments.len() > 2 {
                        return Err(format!(
                            "native `.{}()` expects one or two arguments",
                            property.sym
                        ));
                    }
                    let target_length = self.coerce_primitive_to_number(arguments[0].clone())?;
                    let pad = match arguments.get(1) {
                        Some(pad) => self.coerce_primitive_to_string(pad.clone())?,
                        None => HirExpr::Lit(HirLit::Str(" ".into())),
                    };
                    let suffix = if property.sym == *"padStart" {
                        "pad_start"
                    } else {
                        "pad_end"
                    };
                    let receiver_name = format!("__thaw_pad_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let length_name = format!("__thaw_pad_length_{}", self.next_binding);
                    self.next_binding += 1;
                    let pad_name = format!("__thaw_pad_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(length_name.clone(), HirType::F64);
                    self.scope.insert(pad_name.clone(), HirType::Str);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(pad_name.clone()),
                            HirExpr::Var(length_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((length_name, HirType::F64, target_length));
                    bindings.push((pad_name, HirType::Str, pad));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"toFixed" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::F64, &receiver, "toFixed receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Number.toFixed")?;
                    if arguments.len() > 1 {
                        return Err("native `.toFixed()` expects zero or one argument".into());
                    }
                    let digits = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_to_fixed_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let digits_name = format!("__thaw_to_fixed_digits_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_to_fixed_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::F64);
                    self.scope.insert(digits_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "toFixed() digits argument must be between 0 and 100".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&digits_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Gt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(100.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_fixed".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::F64, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((digits_name, HirType::F64, digits));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if property.sym == *"toPrecision" {
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::F64, &receiver, "toPrecision receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Number.toPrecision")?;
                    if arguments.len() > 1 {
                        return Err("native `.toPrecision()` expects zero or one argument".into());
                    }
                    let receiver_name =
                        format!("__thaw_to_precision_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::F64);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let Some(argument) = arguments.first() else {
                        let mut bindings = vec![(receiver_name.clone(), HirType::F64, receiver)];
                        bindings.extend(spread_bindings);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_string".into())),
                            vec![var(&receiver_name)],
                        );
                        return self.wrap_call_argument_bindings(result, &bindings);
                    };
                    let precision = self.coerce_primitive_to_number(argument.clone())?;
                    let precision_name =
                        format!("__thaw_to_precision_digits_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name =
                        format!("__thaw_to_precision_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(precision_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "toPrecision() argument must be between 1 and 100".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&precision_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(1.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Gt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(100.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_to_precision".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    let mut bindings = vec![(receiver_name, HirType::F64, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((precision_name, HirType::F64, precision));
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if matches!(property.sym.as_ref(), "toLowerCase" | "toUpperCase") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string case receiver")?;
                    let suffix = if property.sym == *"toLowerCase" {
                        "to_lower_case"
                    } else {
                        "to_upper_case"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(property.sym.as_ref(), "isWellFormed" | "toWellFormed") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(
                        &HirType::Str,
                        &receiver,
                        &format!("string {} receiver", property.sym),
                    )?;
                    if property.sym == *"toWellFormed" {
                        return Ok(receiver);
                    }
                    let receiver_name =
                        format!("__thaw_well_formed_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(true)),
                        &[(receiver_name, HirType::Str, receiver)],
                    );
                }
                if property.sym == *"toReversed" {
                    if !call.args.is_empty() {
                        return Err("native `.toReversed()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.toReversed()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_to_reversed".to_string())),
                        vec![receiver],
                    ));
                }
                if matches!(property.sym.as_ref(), "sort" | "toSorted") {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_sort_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), receiver_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if arguments.len() > 1 {
                            return Err(format!(
                                "native `.{}()` expects zero or one comparator",
                                property.sym
                            ));
                        }
                        let result = if let Some(comparator) = arguments.first() {
                            let available = [element_type.clone(), element_type.clone()];
                            let comparator = self.validate_array_callback_value(
                                comparator.clone(),
                                &available,
                                Some(&HirType::F64),
                                "array comparator",
                            )?;
                            self.lower_array_sort_comparator(
                                HirExpr::Var(source_name.clone()),
                                receiver_type.clone(),
                                element_type,
                                comparator,
                                property.sym == *"toSorted",
                            )?
                        } else {
                            let prefix = match &element_type {
                                HirType::F64 => "number",
                                HirType::Str => "string",
                                HirType::Bool => "bool",
                                HirType::Object(_) => "object",
                                other => {
                                    return Err(format!(
                                        "default array sort does not support element type {other:?}"
                                    ))
                                }
                            };
                            let suffix = if property.sym == *"sort" {
                                "sort"
                            } else {
                                "to_sorted"
                            };
                            HirExpr::Call(
                                Box::new(HirExpr::Var(format!(
                                    "__thaw_{prefix}_array_{suffix}"
                                ))),
                                vec![HirExpr::Var(source_name.clone())],
                            )
                        };
                        let mut bindings = vec![(source_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if call.args.len() > 1 {
                        return Err(format!(
                            "native `.{}()` expects zero or one comparator",
                            property.sym
                        ));
                    }
                    if let Some(argument) = call.args.first() {
                        let comparator = self.lower_promise_callback(
                            &argument.expr,
                            &[element_type.clone(), element_type.clone()],
                            Some(&HirType::F64),
                        )?;
                        return self.lower_array_sort_comparator(
                            receiver,
                            receiver_type,
                            element_type,
                            comparator,
                            property.sym == *"toSorted",
                        );
                    }
                    let prefix = match &element_type {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "default array sort does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = if property.sym == *"sort" {
                        "sort"
                    } else {
                        "to_sorted"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(
                    property.sym.as_ref(),
                    "some" | "every" | "find" | "findIndex" | "findLast" | "findLastIndex"
                ) {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_predicate_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(format!(
                                "native `.{}()` expects a predicate and optional thisArg",
                                property.sym
                            ));
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Bool),
                            "array predicate",
                        )?;
                        let result = self.lower_array_predicate_method(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                            match property.sym.as_ref() {
                                "some" => ArrayPredicateMode::Some,
                                "every" => ArrayPredicateMode::Every,
                                "find" => ArrayPredicateMode::Find,
                                "findIndex" => ArrayPredicateMode::FindIndex,
                                "findLast" => ArrayPredicateMode::FindLast,
                                "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                                _ => unreachable!(),
                            },
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a predicate and optional thisArg",
                            property.sym
                        ));
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Bool,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_predicate_method(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                        match property.sym.as_ref() {
                            "some" => ArrayPredicateMode::Some,
                            "every" => ArrayPredicateMode::Every,
                            "find" => ArrayPredicateMode::Find,
                            "findIndex" => ArrayPredicateMode::FindIndex,
                            "findLast" => ArrayPredicateMode::FindLast,
                            "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                            _ => unreachable!(),
                        },
                    );
                }
                if matches!(property.sym.as_ref(), "reduce" | "reduceRight") {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_reduce_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let label = format!("Array.{}", property.sym);
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(format!(
                                "native `.{}()` expects a reducer and optional initial value",
                                property.sym
                            ));
                        }
                        let initial = arguments.get(1).cloned().map(|value| {
                            let ty = self.infer_expr_type(&value)?;
                            Ok::<_, String>((value, ty))
                        }).transpose()?;
                        let accumulator_type = initial
                            .as_ref()
                            .map(|(_, ty)| ty)
                            .unwrap_or(&element_type);
                        let available = [
                            accumulator_type.clone(),
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(accumulator_type),
                            "array reducer",
                        )?;
                        let result = self.lower_array_reduce(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            initial,
                            property.sym == *"reduceRight",
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a reducer and optional initial value",
                            property.sym
                        ));
                    }
                    let initial = call
                        .args
                        .get(1)
                        .map(|argument| {
                            let value = self.lower_expr(&argument.expr)?;
                            let ty = self.infer_expr_type(&value)?;
                            Ok::<_, String>((value, ty))
                        })
                        .transpose()?;
                    let accumulator_type =
                        initial.as_ref().map(|(_, ty)| ty).unwrap_or(&element_type);
                    let callback = self.lower_array_reducer_callback(
                        &call.args[0].expr,
                        accumulator_type,
                        &element_type,
                        &array_type,
                    )?;
                    return self.lower_array_reduce(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        initial,
                        property.sym == *"reduceRight",
                    );
                }
                if property.sym == *"toSpliced" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.toSpliced()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let source_name = format!("__thaw_to_spliced_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.toSpliced")?;
                    for (index, value) in arguments.iter().enumerate() {
                        let expected = if index < 2 {
                            &HirType::F64
                        } else {
                            &element_type
                        };
                        self.expect_type(expected, value, "array toSpliced argument")?;
                    }
                    let result = self.lower_array_to_spliced(
                        HirExpr::Var(source_name.clone()),
                        array_type.clone(),
                        element_type,
                        arguments,
                    )?;
                    let mut bindings = vec![(source_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"at" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "array `.at()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let receiver_name = format!("__thaw_at_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.at")?;
                    let [index] = arguments.as_slice() else {
                        return Err("native array `.at()` expects exactly one index".into());
                    };
                    let index = self.coerce_primitive_to_number(index.clone())?;
                    let result = self.lower_array_at(
                        HirExpr::Var(receiver_name.clone()),
                        array_type.clone(),
                        element_type,
                        index,
                    )?;
                    let mut bindings = vec![(receiver_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"with" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.with()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let receiver_name = format!("__thaw_with_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), array_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.with")?;
                    let [index, value] = arguments.as_slice() else {
                        return Err("native `.with()` expects an index and value".into());
                    };
                    let index = index.clone();
                    self.expect_type(&HirType::F64, &index, "array with index")?;
                    let value = value.clone();
                    self.expect_type(&element_type, &value, "array with value")?;
                    let result = self.lower_array_with(
                        HirExpr::Var(receiver_name.clone()),
                        array_type.clone(),
                        element_type,
                        index,
                        value,
                    )?;
                    let mut bindings = vec![(receiver_name, array_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"flat" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let mut current_type = self.infer_expr_type(&receiver)?;
                    if !matches!(current_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.flat()` requires a homogeneous array, got {current_type:?}"
                        ));
                    }
                    let source_type = current_type.clone();
                    let source_name = format!("__thaw_flat_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), current_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.flat")?;
                    if arguments.len() > 1 {
                        return Err("native `.flat()` expects zero or one depth".into());
                    }
                    let depth = if let Some(argument) = arguments.first() {
                        let value = argument.clone();
                        self.expect_type(&HirType::F64, &value, "array flat depth")?;
                        let constant = match value {
                            HirExpr::Lit(HirLit::F64(value)) => Some(value),
                            HirExpr::BinOp(BinOp::Sub, left, right) => match (*left, *right) {
                                (HirExpr::Lit(HirLit::F64(0.0)), HirExpr::Lit(HirLit::F64(value))) => {
                                    Some(-value)
                                }
                                _ => None,
                            },
                            HirExpr::Call(callee, arguments)
                                if matches!(
                                    callee.as_ref(),
                                    HirExpr::Var(name) if name == "__thaw_number_neg"
                                ) => match arguments.as_slice() {
                                    [HirExpr::Lit(HirLit::F64(value))] => Some(-value),
                                    _ => None,
                                },
                            _ => None,
                        }
                        .ok_or(
                            "native `.flat()` depth must be a numeric literal so its result layout is static",
                        )?;
                        if constant.is_nan() || constant <= 0.0 {
                            0usize
                        } else if constant.is_infinite() {
                            usize::MAX
                        } else {
                            constant.trunc() as usize
                        }
                    } else {
                        1
                    };
                    let mut result = HirExpr::Var(source_name.clone());
                    let mut flattened = false;
                    for _ in 0..depth {
                        let HirType::Array(element) = &current_type else {
                            unreachable!()
                        };
                        let HirType::Array(inner) = element.as_ref() else {
                            break;
                        };
                        let inner = inner.as_ref().clone();
                        result = self.lower_array_flat_one(result, current_type, inner.clone())?;
                        current_type = HirType::Array(Box::new(inner));
                        flattened = true;
                    }
                    if !flattened {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_slice".into())),
                            vec![
                                result,
                                HirExpr::Lit(HirLit::F64(0.0)),
                                HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                            ],
                        );
                    }
                    let mut bindings = vec![(source_name, source_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"flatMap" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.flatMap()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_flat_map_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.flatMap")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.flatMap()` expects a callback and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array mapper",
                        )?;
                        let mapped = self.lower_array_map(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mapped_type = self.infer_expr_type(&mapped)?;
                        let HirType::Array(mapped_element) = &mapped_type else {
                            unreachable!("array map always returns an array")
                        };
                        let HirType::Array(flat_element) = mapped_element.as_ref() else {
                            return Err(format!(
                                "native `.flatMap()` callback must return a homogeneous array, got {mapped_element:?}"
                            ));
                        };
                        let flat_element = flat_element.as_ref().clone();
                        let result = self.lower_array_flat_one(
                            mapped,
                            mapped_type,
                            flat_element,
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.flatMap()` expects a callback and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    let mapped = self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    )?;
                    let mapped_type = self.infer_expr_type(&mapped)?;
                    let HirType::Array(mapped_element) = &mapped_type else {
                        unreachable!("array map always returns an array")
                    };
                    let HirType::Array(flat_element) = mapped_element.as_ref() else {
                        return Err(format!(
                            "native `.flatMap()` callback must return a homogeneous array, got {mapped_element:?}"
                        ));
                    };
                    let flat_element = flat_element.as_ref().clone();
                    return self.lower_array_flat_one(mapped, mapped_type, flat_element);
                }
                if property.sym == *"map" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.map()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_map_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.map")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.map()` expects a callback and optional thisArg".into(),
                            );
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            None,
                            "array mapper",
                        )?;
                        let this_arg = arguments.get(1).cloned();
                        let result = self.lower_array_map(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            this_arg,
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.map()` expects a callback and optional thisArg".into()
                        );
                    }
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"filter" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.filter()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_filter_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.filter")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.filter()` expects a predicate and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Bool),
                            "array predicate",
                        )?;
                        let result = self.lower_array_filter(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.filter()` expects a predicate and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Bool,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_filter(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"forEach" && self.receiver_is_map_or_set(&member.obj) {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (key_type, value_type, keys_double_as_values) = match &receiver_type {
                        HirType::Map(key_type, value_type) => {
                            (key_type.as_ref().clone(), value_type.as_ref().clone(), false)
                        }
                        HirType::Set(element_type) => {
                            let element_type = element_type.as_ref().clone();
                            (element_type.clone(), element_type, true)
                        }
                        _ => unreachable!("receiver_is_map_or_set confirmed this above"),
                    };
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err(
                            "native `.forEach()` does not support spread arguments on a Map/Set"
                                .into(),
                        );
                    }
                    let [argument] = call.args.as_slice() else {
                        return Err("native `.forEach()` expects exactly one argument".into());
                    };
                    let arity = match argument.expr.as_ref() {
                        Expr::Arrow(arrow) => arrow.params.len(),
                        Expr::Fn(function) => function.function.params.len(),
                        Expr::Ident(ident) => {
                            let name = self.resolve_binding(ident.sym.as_ref());
                            let HirType::Function(params, _) = self
                                .scope
                                .get(&name)
                                .ok_or_else(|| format!("unknown Map/Set forEach callback `{name}`"))?
                            else {
                                return Err(format!(
                                    "Map/Set forEach callback `{name}` is not a function value"
                                ));
                            };
                            params.len()
                        }
                        _ => {
                            return Err(
                                "Map/Set forEach callback must be an arrow or function value"
                                    .into(),
                            )
                        }
                    };
                    if arity > 3 {
                        return Err(format!(
                            "Map/Set forEach callback accepts at most three parameters, got {arity}"
                        ));
                    }
                    let available = [value_type.clone(), key_type.clone(), receiver_type.clone()];
                    let callback = self.lower_promise_callback(
                        &argument.expr,
                        &available[..arity],
                        Some(&HirType::Void),
                    )?;
                    let callback_type = self.infer_expr_type(&callback)?;
                    let HirType::Function(params, _) = &callback_type else {
                        unreachable!("Map/Set forEach callback was validated as a function")
                    };
                    let receiver_name = format!("__thaw_map_for_each_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let callback_name = format!("__thaw_map_for_each_callback_{}", self.next_binding);
                    self.next_binding += 1;
                    let keys_name = format!("__thaw_map_for_each_keys_{}", self.next_binding);
                    self.next_binding += 1;
                    let values_name = format!("__thaw_map_for_each_values_{}", self.next_binding);
                    self.next_binding += 1;
                    let length_name = format!("__thaw_map_for_each_length_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_map_for_each_index_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_var_name = format!("__thaw_map_for_each_key_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_var_name = format!("__thaw_map_for_each_value_{}", self.next_binding);
                    self.next_binding += 1;
                    let keys_array_type = HirType::Array(Box::new(key_type.clone()));
                    let values_array_type = HirType::Array(Box::new(value_type.clone()));
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(callback_name.clone(), callback_type.clone());
                    self.scope.insert(keys_name.clone(), keys_array_type.clone());
                    self.scope.insert(values_name.clone(), values_array_type.clone());
                    self.scope.insert(length_name.clone(), HirType::F64);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    self.scope.insert(key_var_name.clone(), key_type.clone());
                    self.scope.insert(value_var_name.clone(), value_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let available_vars = [
                        var(&value_var_name),
                        var(&key_var_name),
                        var(&receiver_name),
                    ];
                    let callback_call = HirExpr::Call(
                        Box::new(var(&callback_name)),
                        available_vars[..params.len()].to_vec(),
                    );
                    let values_source_name = if keys_double_as_values {
                        keys_name.clone()
                    } else {
                        values_name.clone()
                    };
                    let mut body_stmts = vec![HirStmt::Let(
                        keys_name.clone(),
                        keys_array_type,
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                            vec![var(&receiver_name)],
                        ),
                    )];
                    if !keys_double_as_values {
                        body_stmts.push(HirStmt::Let(
                            values_name,
                            values_array_type,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_map_snapshot_values".to_string())),
                                vec![var(&receiver_name)],
                            ),
                        ));
                    }
                    body_stmts.extend([
                        HirStmt::Let(
                            length_name.clone(),
                            HirType::F64,
                            HirExpr::ArrayLen(Box::new(var(&keys_name))),
                        ),
                        HirStmt::Let(index_name.clone(), HirType::F64, HirExpr::Lit(HirLit::F64(0.0))),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&index_name)),
                                Box::new(var(&length_name)),
                            ),
                            vec![
                                HirStmt::Let(
                                    key_var_name,
                                    key_type.clone(),
                                    HirExpr::TypedIndex(
                                        Box::new(var(&keys_name)),
                                        Box::new(var(&index_name)),
                                        key_type,
                                    ),
                                ),
                                HirStmt::Let(
                                    value_var_name,
                                    value_type.clone(),
                                    HirExpr::TypedIndex(
                                        Box::new(var(&values_source_name)),
                                        Box::new(var(&index_name)),
                                        value_type,
                                    ),
                                ),
                                HirStmt::Expr(callback_call),
                                HirStmt::Expr(HirExpr::Assign(
                                    index_name.clone(),
                                    Box::new(HirExpr::BinOp(
                                        BinOp::Add,
                                        Box::new(var(&index_name)),
                                        Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                                    )),
                                )),
                            ],
                        ),
                        HirStmt::Return(None),
                    ]);
                    let body = HirExpr::Block(body_stmts);
                    let bindings = vec![
                        (receiver_name, receiver_type, receiver),
                        (callback_name, callback_type, callback),
                    ];
                    return self.wrap_call_argument_bindings(body, &bindings);
                }
                if property.sym == *"forEach" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.forEach()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        let source_name = format!("__thaw_for_each_source_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(source_name.clone(), array_type.clone());
                        let (arguments, spread_bindings) =
                            self.lower_native_spread_values(&call.args, "Array.forEach")?;
                        if !(1..=2).contains(&arguments.len()) {
                            return Err(
                                "native `.forEach()` expects a callback and optional thisArg"
                                    .into(),
                            );
                        }
                        let available = [
                            element_type.clone(),
                            HirType::F64,
                            array_type.clone(),
                        ];
                        let callback = self.validate_array_callback_value(
                            arguments[0].clone(),
                            &available,
                            Some(&HirType::Void),
                            "array callback",
                        )?;
                        let result = self.lower_array_for_each(
                            HirExpr::Var(source_name.clone()),
                            array_type.clone(),
                            element_type,
                            callback,
                            arguments.get(1).cloned(),
                        )?;
                        let mut bindings = vec![(source_name, array_type, receiver)];
                        bindings.extend(spread_bindings);
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.forEach()` expects a callback and optional thisArg".into(),
                        );
                    }
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Void,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_for_each(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"slice" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.slice()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.slice")?;
                    if arguments.len() > 2 {
                        return Err("native `.slice()` expects zero to two arguments".into());
                    }
                    let mut indices = Vec::with_capacity(2);
                    for value in arguments {
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_slice_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_slice_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_slice".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"copyWithin" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.copyWithin()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.copyWithin")?;
                    if !(2..=3).contains(&arguments.len()) {
                        return Err("native `.copyWithin()` expects two or three arguments".into());
                    }
                    let mut indices = Vec::with_capacity(3);
                    for value in arguments {
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.len() == 2 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_copy_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_copy_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_copy_within".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"fill" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.fill()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    };
                    let element = element.as_ref().clone();
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.fill")?;
                    if !(1..=3).contains(&arguments.len()) {
                        return Err("native `.fill()` expects one to three arguments".into());
                    }
                    let value = arguments[0].clone();
                    self.expect_type(&element, &value, "fill value")?;
                    let mut indices = Vec::with_capacity(2);
                    for argument in arguments.into_iter().skip(1) {
                        indices.push(self.coerce_primitive_to_number(argument)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let runtime = match &element {
                        HirType::F64 => "__thaw_number_array_fill",
                        HirType::Bool => "__thaw_bool_array_fill",
                        HirType::Str | HirType::Array(_) | HirType::Object(_) => {
                            "__thaw_pointer_array_fill"
                        }
                        _ => "__thaw_array_fill",
                    };
                    let receiver_name = format!("__thaw_fill_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let value_name = format!("__thaw_fill_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), element.clone());
                    let mut bindings = vec![
                        (receiver_name.clone(), receiver_type, receiver),
                    ];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name.clone(), element, value));
                    let mut arguments = vec![HirExpr::Var(receiver_name), HirExpr::Var(value_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_fill_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), arguments);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"reverse" {
                    if !call.args.is_empty() {
                        return Err("native `.reverse()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.reverse()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_reverse".to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"join" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let source_name = format!("__thaw_join_source_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(source_name.clone(), receiver_type.clone());
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Array.join")?;
                    if arguments.len() > 1 {
                        return Err("native `.join()` expects zero or one argument".into());
                    }
                    let separator = if let Some(argument) = arguments.first() {
                        self.coerce_primitive_to_string(argument.clone())?
                    } else {
                        HirExpr::Lit(HirLit::Str(",".to_string()))
                    };
                    let result = match receiver_type.clone() {
                        HirType::Array(element) => {
                            let array_type = HirType::Array(element.clone());
                            let builtin = match element.as_ref() {
                                HirType::F64 => "__thaw_number_array_join",
                                HirType::Str => "__thaw_string_array_join",
                                HirType::Bool => "__thaw_bool_array_join",
                                HirType::Object(_) => "__thaw_object_array_join",
                                other => {
                                    return Err(format!(
                                        "array join does not support element type {other:?}"
                                    ))
                                }
                            };
                            let receiver_name =
                                format!("__thaw_join_receiver_{}", self.next_binding);
                            self.next_binding += 1;
                            let separator_name =
                                format!("__thaw_join_separator_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(receiver_name.clone(), array_type.clone());
                            self.scope.insert(separator_name.clone(), HirType::Str);
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(builtin.to_string())),
                                vec![
                                    HirExpr::Var(receiver_name.clone()),
                                    HirExpr::Var(separator_name.clone()),
                                ],
                            );
                            self.wrap_call_argument_bindings(
                                result,
                                &[
                                    (
                                        receiver_name,
                                        array_type,
                                        HirExpr::Var(source_name.clone()),
                                    ),
                                    (separator_name, HirType::Str, separator),
                                ],
                            )
                        }
                        HirType::Tuple(elements) => self.join_tuple(
                            HirExpr::Var(source_name.clone()),
                            elements,
                            separator,
                        ),
                        other => Err(format!(
                            "`.join()` requires an array receiver, got {other:?}"
                        )),
                    }?;
                    let mut bindings = vec![(source_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(
                    property.sym.as_ref(),
                    "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith"
                ) {
                    let label = format!("native .{}", property.sym);
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, &label)?;
                    if !(1..=2).contains(&arguments.len()) {
                        return Err(format!(
                            "native `.{}` expects one or two arguments",
                            property.sym
                        ));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == HirType::Str {
                        let needle = self.coerce_primitive_to_string(arguments[0].clone())?;
                        let position = if let Some(argument) = arguments.get(1) {
                            self.coerce_primitive_to_number(argument.clone())?
                        } else if property.sym == *"endsWith" || property.sym == *"lastIndexOf" {
                            HirExpr::Lit(HirLit::F64(f64::INFINITY))
                        } else {
                            HirExpr::Lit(HirLit::F64(0.0))
                        };
                        let suffix = match property.sym.as_ref() {
                            "indexOf" => "index_of",
                            "lastIndexOf" => "last_index_of",
                            "includes" => "includes",
                            "startsWith" => "starts_with",
                            "endsWith" => "ends_with",
                            _ => unreachable!(),
                        };
                        let receiver_name =
                            format!("__thaw_string_search_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name =
                            format!("__thaw_string_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let position_name =
                            format!("__thaw_string_search_position_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(needle_name.clone(), HirType::Str);
                        self.scope.insert(position_name.clone(), HirType::F64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(needle_name.clone()),
                                HirExpr::Var(position_name.clone()),
                            ],
                        );
                        let mut bindings = vec![(receiver_name, HirType::Str, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, HirType::Str, needle));
                        bindings.push((position_name, HirType::F64, position));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    if matches!(property.sym.as_ref(), "startsWith" | "endsWith") {
                        return Err(format!(
                            "`.{}` requires a string receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    let HirType::Array(element) = receiver_type.clone() else {
                        return Err(format!(
                            "`.{}` requires a homogeneous array receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let needle = arguments[0].clone();
                    let needle_type = self.infer_expr_type(&needle)?;
                    let from_index = if let Some(argument) = arguments.get(1) {
                        self.coerce_primitive_to_number(argument.clone())?
                    } else if property.sym == *"lastIndexOf" {
                        HirExpr::Lit(HirLit::F64(f64::INFINITY))
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    if needle_type != *element {
                        let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let start_name = format!("__thaw_search_start_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(needle_name.clone(), needle_type.clone());
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let result = if property.sym == *"includes" {
                            HirExpr::Lit(HirLit::Bool(false))
                        } else {
                            HirExpr::Lit(HirLit::F64(-1.0))
                        };
                        let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                        bindings.extend(spread_bindings);
                        bindings.push((needle_name, needle_type, needle));
                        bindings.push((start_name, HirType::F64, from_index));
                        return self.wrap_call_argument_bindings(result, &bindings);
                    }
                    let prefix = match element.as_ref() {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "array search does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = match property.sym.as_ref() {
                        "includes" => "includes",
                        "lastIndexOf" => "last_index_of",
                        _ => "index_of",
                    };
                    let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                    self.next_binding += 1;
                    let start_name = format!("__thaw_search_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(needle_name.clone(), needle_type.clone());
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(needle_name.clone()),
                            HirExpr::Var(start_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((needle_name, needle_type, needle));
                    bindings.push((start_name, HirType::F64, from_index));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"getTime" {
                    if !call.args.is_empty() {
                        return Err("native `.getTime()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.getTime receiver")?;
                    return Ok(HirExpr::PropAccess(
                        Box::new(receiver),
                        date_type,
                        "timestamp".to_string(),
                    ));
                }
                if property.sym == *"setTime" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.setTime receiver")?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Date.setTime")?;
                    let [value] = arguments.as_slice() else {
                        return Err("native `.setTime()` expects exactly one argument".into());
                    };
                    let value = self.coerce_primitive_to_number(value.clone())?;
                    let receiver_name =
                        format!("__thaw_date_set_time_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_date_set_time_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    self.scope.insert(value_name.clone(), HirType::F64);
                    let result = HirExpr::PropAssign(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        date_type.clone(),
                        "timestamp".to_string(),
                        Box::new(HirExpr::Var(value_name.clone())),
                    );
                    let mut bindings = vec![(receiver_name, date_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((value_name, HirType::F64, value));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                // Each entry pairs a setter's native intrinsic with, per
                // parameter position, the getter intrinsic that supplies
                // that position's default when the caller omits it -- `None`
                // marks the one leading parameter every setter requires.
                let date_setter = match property.sym.as_ref() {
                    "setFullYear" | "setUTCFullYear" => Some((
                        "__thaw_date_set_full_year",
                        vec![None, Some("__thaw_date_get_month"), Some("__thaw_date_get_date")],
                    )),
                    "setMonth" | "setUTCMonth" => Some((
                        "__thaw_date_set_month",
                        vec![None, Some("__thaw_date_get_date")],
                    )),
                    "setDate" | "setUTCDate" => Some(("__thaw_date_set_date", vec![None])),
                    "setHours" | "setUTCHours" => Some((
                        "__thaw_date_set_hours",
                        vec![
                            None,
                            Some("__thaw_date_get_minutes"),
                            Some("__thaw_date_get_seconds"),
                            Some("__thaw_date_get_milliseconds"),
                        ],
                    )),
                    "setMinutes" | "setUTCMinutes" => Some((
                        "__thaw_date_set_minutes",
                        vec![
                            None,
                            Some("__thaw_date_get_seconds"),
                            Some("__thaw_date_get_milliseconds"),
                        ],
                    )),
                    "setSeconds" | "setUTCSeconds" => Some((
                        "__thaw_date_set_seconds",
                        vec![None, Some("__thaw_date_get_milliseconds")],
                    )),
                    "setMilliseconds" | "setUTCMilliseconds" => {
                        Some(("__thaw_date_set_milliseconds", vec![None]))
                    }
                    _ => None,
                };
                if let Some((intrinsic, param_defaults)) = date_setter {
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(
                        &date_type,
                        &receiver,
                        &format!("Date.{} receiver", property.sym),
                    )?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Date.{}", property.sym),
                    )?;
                    if arguments.is_empty() || arguments.len() > param_defaults.len() {
                        return Err(format!(
                            "native `.{}()` expects one to {} argument(s)",
                            property.sym,
                            param_defaults.len()
                        ));
                    }
                    let receiver_name =
                        format!("__thaw_date_set_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), date_type.clone(), receiver)];
                    bindings.extend(spread_bindings);
                    let mut param_vars = Vec::new();
                    for (index, getter) in param_defaults.iter().enumerate() {
                        let value = if let Some(argument) = arguments.get(index) {
                            self.coerce_primitive_to_number(argument.clone())?
                        } else {
                            let getter =
                                getter.expect("the leading parameter is always required");
                            let timestamp = HirExpr::PropAccess(
                                Box::new(HirExpr::Var(receiver_name.clone())),
                                date_type.clone(),
                                "timestamp".to_string(),
                            );
                            HirExpr::Call(
                                Box::new(HirExpr::Var(getter.to_string())),
                                vec![timestamp],
                            )
                        };
                        let param_name =
                            format!("__thaw_date_set_arg_{index}_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(param_name.clone(), HirType::F64);
                        bindings.push((param_name.clone(), HirType::F64, value));
                        param_vars.push(HirExpr::Var(param_name));
                    }
                    let timestamp = HirExpr::PropAccess(
                        Box::new(HirExpr::Var(receiver_name.clone())),
                        date_type.clone(),
                        "timestamp".to_string(),
                    );
                    let mut call_args = vec![timestamp];
                    call_args.extend(param_vars);
                    let new_timestamp =
                        HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), call_args);
                    let result = HirExpr::PropAssign(
                        Box::new(HirExpr::Var(receiver_name)),
                        date_type,
                        "timestamp".to_string(),
                        Box::new(new_timestamp),
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                let date_getter_intrinsic = match property.sym.as_ref() {
                    "getFullYear" | "getUTCFullYear" => Some("__thaw_date_get_full_year"),
                    "getMonth" | "getUTCMonth" => Some("__thaw_date_get_month"),
                    "getDate" | "getUTCDate" => Some("__thaw_date_get_date"),
                    "getDay" | "getUTCDay" => Some("__thaw_date_get_day"),
                    "getHours" | "getUTCHours" => Some("__thaw_date_get_hours"),
                    "getMinutes" | "getUTCMinutes" => Some("__thaw_date_get_minutes"),
                    "getSeconds" | "getUTCSeconds" => Some("__thaw_date_get_seconds"),
                    "getMilliseconds" | "getUTCMilliseconds" => {
                        Some("__thaw_date_get_milliseconds")
                    }
                    _ => None,
                };
                if let Some(intrinsic) = date_getter_intrinsic {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(
                        &date_type,
                        &receiver,
                        &format!("Date.{} receiver", property.sym),
                    )?;
                    let timestamp = HirExpr::PropAccess(
                        Box::new(receiver),
                        date_type,
                        "timestamp".to_string(),
                    );
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
                        vec![timestamp],
                    ));
                }
                if property.sym == *"toISOString" {
                    if !call.args.is_empty() {
                        return Err("native `.toISOString()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.toISOString receiver")?;
                    let receiver_name =
                        format!("__thaw_date_iso_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_date_iso_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let timestamp = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        date_type.clone(),
                        "timestamp".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_to_iso_string".into())),
                                vec![timestamp],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                                "Invalid time value".into(),
                            )))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(var(&raw_name))),
                    ]);
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            HirType::Str,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let bindings = vec![(receiver_name, date_type, receiver)];
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"get" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Map(key_type, value_type) = &receiver_type else {
                        return Err(format!(
                            "native `.get()` requires a Map receiver, got {receiver_type:?}"
                        ));
                    };
                    let key_type = key_type.as_ref().clone();
                    let value_type = value_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (value_suffix, needs_type_wrap) = map_value_get_suffix(&value_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Map.get")?;
                    let [key] = arguments.as_slice() else {
                        return Err("native `.get()` expects exactly one argument".into());
                    };
                    let key = if key_type == HirType::F64 {
                        self.coerce_primitive_to_number(key.clone())?
                    } else {
                        self.coerce_primitive_to_string(key.clone())?
                    };
                    let receiver_name = format!("__thaw_map_get_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_get_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    let var = |name: &str| HirExpr::Var(name.into());
                    let has_intrinsic = format!("__thaw_map_{key_suffix}_has");
                    let get_intrinsic = format!("__thaw_map_{key_suffix}_get_{value_suffix}");
                    let raw_get = HirExpr::Call(
                        Box::new(HirExpr::Var(get_intrinsic)),
                        vec![var(&receiver_name), var(&key_name)],
                    );
                    let decoded = if needs_type_wrap {
                        HirExpr::TypedClosure(value_type.clone(), Box::new(raw_get))
                    } else {
                        raw_get
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var(has_intrinsic)),
                                vec![var(&receiver_name), var(&key_name)],
                            ),
                            Vec::new(),
                            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                                value_type.clone(),
                            )))],
                        ),
                        HirStmt::Return(Some(HirExpr::OptionalSome(
                            Box::new(decoded),
                            value_type.clone(),
                        ))),
                    ]);
                    let result_type = HirType::Optional(Box::new(value_type));
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"set" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Map(key_type, value_type) = &receiver_type else {
                        return Err(format!(
                            "native `.set()` requires a Map receiver, got {receiver_type:?}"
                        ));
                    };
                    let key_type = key_type.as_ref().clone();
                    let value_type = value_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Map.set")?;
                    let [key, value] = arguments.as_slice() else {
                        return Err("native `.set()` expects exactly two arguments".into());
                    };
                    let key = if key_type == HirType::F64 {
                        self.coerce_primitive_to_number(key.clone())?
                    } else {
                        self.coerce_primitive_to_string(key.clone())?
                    };
                    self.expect_type(&value_type, value, "Map.set value")?;
                    let receiver_name = format!("__thaw_map_set_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_set_key_{}", self.next_binding);
                    self.next_binding += 1;
                    let value_name = format!("__thaw_map_set_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    self.scope.insert(value_name.clone(), value_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(key_name.clone()),
                            HirExpr::Var(value_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    bindings.push((value_name, value_type, value.clone()));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"add" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Set(element_type) = &receiver_type else {
                        return Err(format!(
                            "native `.add()` requires a Set receiver, got {receiver_type:?}"
                        ));
                    };
                    let element_type = element_type.as_ref().clone();
                    let key_suffix = map_key_intrinsic_suffix(&element_type)?;
                    let (arguments, spread_bindings) =
                        self.lower_native_spread_values(&call.args, "Set.add")?;
                    let [element] = arguments.as_slice() else {
                        return Err("native `.add()` expects exactly one argument".into());
                    };
                    let element = if element_type == HirType::F64 {
                        self.coerce_primitive_to_number(element.clone())?
                    } else {
                        self.coerce_primitive_to_string(element.clone())?
                    };
                    let receiver_name = format!("__thaw_set_add_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let element_name = format!("__thaw_set_add_element_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(element_name.clone(), element_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_map_{key_suffix}_set"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(element_name.clone()),
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((element_name, element_type, element));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "has" | "delete") {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let key_type = match &receiver_type {
                        HirType::Map(key_type, _) => key_type.as_ref().clone(),
                        HirType::Set(element_type) => element_type.as_ref().clone(),
                        other => {
                            return Err(format!(
                                "native `.{}()` requires a Map or Set receiver, got {other:?}",
                                property.sym
                            ))
                        }
                    };
                    let key_suffix = map_key_intrinsic_suffix(&key_type)?;
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Map/Set.{}", property.sym),
                    )?;
                    let [key] = arguments.as_slice() else {
                        return Err(format!(
                            "native `.{}()` expects exactly one argument",
                            property.sym
                        ));
                    };
                    let key = if key_type == HirType::F64 {
                        self.coerce_primitive_to_number(key.clone())?
                    } else {
                        self.coerce_primitive_to_string(key.clone())?
                    };
                    let intrinsic = if property.sym == *"has" {
                        format!("__thaw_map_{key_suffix}_has")
                    } else {
                        format!("__thaw_map_{key_suffix}_delete")
                    };
                    let receiver_name =
                        format!("__thaw_map_set_query_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let key_name = format!("__thaw_map_set_query_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(key_name.clone(), key_type.clone());
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic)),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(key_name.clone()),
                        ],
                    );
                    let mut bindings = vec![(receiver_name, receiver_type, receiver)];
                    bindings.extend(spread_bindings);
                    bindings.push((key_name, key_type, key));
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"clear" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Map(_, _) | HirType::Set(_)) {
                        return Err(format!(
                            "native `.clear()` requires a Map or Set receiver, got {receiver_type:?}"
                        ));
                    }
                    if !call.args.is_empty() {
                        return Err("native `.clear()` expects no arguments".into());
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_map_clear".to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"keys" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let key_type = match &receiver_type {
                        HirType::Map(key_type, _) => key_type.as_ref().clone(),
                        HirType::Set(element_type) => element_type.as_ref().clone(),
                        other => {
                            return Err(format!(
                                "native `.keys()` requires a Map or Set receiver, got {other:?}"
                            ))
                        }
                    };
                    if !call.args.is_empty() {
                        return Err("native `.keys()` expects no arguments".into());
                    }
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(key_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_map_snapshot_keys".to_string())),
                            vec![receiver],
                        )),
                    ));
                }
                if property.sym == *"values" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (value_type, intrinsic) = match &receiver_type {
                        HirType::Map(_, value_type) => {
                            (value_type.as_ref().clone(), "__thaw_map_snapshot_values")
                        }
                        // A Set's elements ARE its "values" -- it has no
                        // separate value to snapshot.
                        HirType::Set(element_type) => {
                            (element_type.as_ref().clone(), "__thaw_map_snapshot_keys")
                        }
                        other => {
                            return Err(format!(
                                "native `.values()` requires a Map or Set receiver, got {other:?}"
                            ))
                        }
                    };
                    if !call.args.is_empty() {
                        return Err("native `.values()` expects no arguments".into());
                    }
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(value_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![receiver],
                        )),
                    ));
                }
                if property.sym == *"entries" {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let (pair_type, intrinsic) = match &receiver_type {
                        HirType::Map(key_type, value_type) => (
                            HirType::Tuple(vec![key_type.as_ref().clone(), value_type.as_ref().clone()]),
                            "__thaw_map_snapshot_entries",
                        ),
                        HirType::Set(element_type) => (
                            HirType::Tuple(vec![element_type.as_ref().clone(), element_type.as_ref().clone()]),
                            "__thaw_set_snapshot_entries",
                        ),
                        other => {
                            return Err(format!(
                                "native `.entries()` requires a Map or Set receiver, got {other:?}"
                            ))
                        }
                    };
                    if !call.args.is_empty() {
                        return Err("native `.entries()` expects no arguments".into());
                    }
                    return Ok(HirExpr::TypedClosure(
                        HirType::Array(Box::new(pair_type)),
                        Box::new(HirExpr::Call(
                            Box::new(HirExpr::Var(intrinsic.to_string())),
                            vec![receiver],
                        )),
                    ));
                }
                if property.sym == *"toJSON" {
                    if !call.args.is_empty() {
                        return Err("native `.toJSON()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(&date_type, &receiver, "Date.toJSON receiver")?;
                    let receiver_name = format!("__thaw_date_json_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let raw_name = format!("__thaw_date_json_raw_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), date_type.clone());
                    self.scope.insert(raw_name.clone(), HirType::Str);
                    let var = |name: &str| HirExpr::Var(name.into());
                    let timestamp = HirExpr::PropAccess(
                        Box::new(var(&receiver_name)),
                        date_type.clone(),
                        "timestamp".to_string(),
                    );
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(
                            raw_name.clone(),
                            HirType::Str,
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_date_to_iso_string".into())),
                                vec![timestamp],
                            ),
                        ),
                        HirStmt::If(
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_string_is_null".into())),
                                vec![var(&raw_name)],
                            ),
                            vec![HirStmt::Return(Some(HirExpr::NullableNone(HirType::Str)))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::NullableSome(
                            Box::new(var(&raw_name)),
                            HirType::Str,
                        ))),
                    ]);
                    let result_type = HirType::Nullable(Box::new(HirType::Str));
                    let mut referenced = BTreeSet::new();
                    collect_referenced_bindings(&body, &mut referenced);
                    let captures = referenced
                        .into_iter()
                        .filter_map(|captured| {
                            self.scope
                                .get(&captured)
                                .cloned()
                                .map(|ty| HirParam { name: captured, ty })
                        })
                        .collect();
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Lambda(
                            captures,
                            Vec::new(),
                            result_type,
                            Box::new(body),
                        )),
                        Vec::new(),
                    );
                    let bindings = vec![(receiver_name, date_type, receiver)];
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(
                    property.sym.as_ref(),
                    "toDateString" | "toTimeString" | "toUTCString"
                ) {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    self.expect_type(
                        &date_type,
                        &receiver,
                        &format!("Date.{} receiver", property.sym),
                    )?;
                    let intrinsic = match property.sym.as_ref() {
                        "toDateString" => "__thaw_date_to_date_string",
                        "toTimeString" => "__thaw_date_to_time_string",
                        "toUTCString" => "__thaw_date_to_utc_string",
                        _ => unreachable!(),
                    };
                    let timestamp = HirExpr::PropAccess(
                        Box::new(receiver),
                        date_type,
                        "timestamp".to_string(),
                    );
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(intrinsic.to_string())),
                        vec![timestamp],
                    ));
                }
                if property.sym == *"toString" {
                    if !call.args.is_empty() {
                        return Err("native `.toString()` does not accept arguments yet".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let date_type = date_object_type();
                    if self.infer_expr_type(&receiver)? == date_type {
                        let timestamp = HirExpr::PropAccess(
                            Box::new(receiver),
                            date_type,
                            "timestamp".to_string(),
                        );
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_date_to_string".to_string())),
                            vec![timestamp],
                        ));
                    }
                    return self.coerce_primitive_to_string(receiver);
                }
                if property.sym == *"valueOf" {
                    if !call.args.is_empty() {
                        return Err("native `.valueOf()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == date_object_type() {
                        return Ok(HirExpr::PropAccess(
                            Box::new(receiver),
                            receiver_type,
                            "timestamp".to_string(),
                        ));
                    }
                    if !matches!(receiver_type, HirType::F64 | HirType::Str | HirType::Bool) {
                        return Err(format!(
                            "native `.valueOf()` requires a number, string or boolean receiver, got {receiver_type:?}"
                        ));
                    }
                    return Ok(receiver);
                }
        unreachable!("native instance builtin dispatch was checked before lowering")
    }
}
