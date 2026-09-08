impl<'a> FnLowerer<'a> {
    fn wrap_call_argument_bindings(
        &mut self,
        mut result: HirExpr,
        bindings: &[(Symbol, HirType, HirExpr)],
    ) -> Result<HirExpr, String> {
        if bindings.is_empty() {
            return Ok(result);
        }
        let result_type = self.infer_expr_type(&result)?;
        for index in (0..bindings.len()).rev() {
            let (name, ty, source) = &bindings[index];
            let body = if result_type == HirType::Void {
                if matches!(&result, HirExpr::Block(_)) {
                    result
                } else {
                    HirExpr::Block(vec![HirStmt::Expr(result)])
                }
            } else {
                result
            };
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter(|referenced| referenced != name)
                .filter(|referenced| {
                    bindings
                        .iter()
                        .position(|(binding, _, _)| binding == referenced)
                        .is_none_or(|position| position < index)
                })
                .filter_map(|referenced| {
                    self.scope.get(&referenced).cloned().map(|ty| HirParam {
                        name: referenced,
                        ty,
                    })
                })
                .collect();
            result = HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    vec![HirParam {
                        name: name.clone(),
                        ty: ty.clone(),
                    }],
                    result_type.clone(),
                    Box::new(body),
                )),
                vec![source.clone()],
            );
        }
        Ok(result)
    }

    /// Lowers `receiver.property(args...)` where `receiver`'s own type is
    /// `HirType::JsValue` -- an opaque handle with no compiled class/method
    /// table -- into a call to `callDynamicMethod` (or its sibling
    /// `callDynamicMethodHandle`, see below), existing low-level
    /// intrinsics (`getDynamicValue`/`callDynamicValueHandle`'s siblings)
    /// that call a named method on a retained JS value by hex-decoding
    /// nothing at all: they just take the handle, the method name, and a
    /// JSON-encoded argument array (see `compile_call_dynamic_method`/
    /// `compile_call_dynamic_method_handle` in thaw-llvm's
    /// `dynamic_host.rs`). Building that argument array reuses
    /// `coerce_to_declared`'s existing native-value-to-`Json` laundering
    /// twice: once per argument (so a `JsValue` argument -- e.g. passing
    /// one schema into another's method -- flows through the same
    /// handle-id placeholder this session's earlier fix already wired
    /// up), then once more over the whole homogeneous-`Json` array
    /// literal (so it becomes one real JSON array value, not a native
    /// Thaw array of boxed `Json` pointers).
    ///
    /// A dynamic method's own result could itself be plain data (real
    /// example: zod's `schema.safeParse({ name: "Alice", age: 30 })`,
    /// returning `{ success, data/error }`) or another live `JsValue`
    /// (a hypothetical chained schema-builder method returning another
    /// schema instance) -- thaw has no compiled knowledge of a dynamic
    /// method's real shape either way, so it can't tell which just from
    /// the call site. Resolved the same way an ambiguous `let`/`const`
    /// declaration's own initializer already is elsewhere: `expected`,
    /// this call's own expected-type hint (`None` unless the immediate
    /// enclosing declaration carries an explicit `: JsValue` annotation),
    /// picks `callDynamicMethodHandle` when it says so and
    /// `callDynamicMethod` (the JSON-returning default, matching every
    /// caller before this existed) otherwise.
    /// Infers a member-call receiver's static type well enough to pick a
    /// dispatch strategy (a known native class's method table, a
    /// dynamic-`JsValue` method call, or neither) -- extracted out of
    /// `lower_call`'s own member-call branch so the `Expr::Call` arm
    /// below can recurse into itself for a receiver that's *itself* a
    /// chained method call (see that arm's own doc comment).
    fn infer_member_receiver_type(&self, expr: &Expr) -> Option<HirType> {
        match expr {
            Expr::Ident(receiver) => {
                let name = self.resolve_binding(receiver.sym.as_ref());
                self.scope
                    .get(&name)
                    .map(|ty| {
                        if *ty == HirType::Dynamic {
                            HirType::JsValue
                        } else {
                            ty.clone()
                        }
                    })
                    .or_else(|| {
                        matches!(receiver.sym.as_ref(), "crypto" | "process")
                            .then_some(HirType::JsValue)
                    })
            }
            Expr::New(construction) => construction
                .callee
                .as_ident()
                .and_then(|class| self.interfaces.get(class.sym.as_ref()).cloned()),
            Expr::This(_) => self.scope.get(&self.resolve_binding("this")).cloned(),
            Expr::Member(member) => infer_generic_constructor_expr_type(
                expr,
                self.interfaces,
                self.generic_interfaces,
                std::slice::from_ref(&self.scope),
                &self.generic_call_returns,
            )
            .ok()
            .or_else(|| {
                (self.infer_member_receiver_type(&member.obj) == Some(HirType::JsValue))
                    .then_some(HirType::JsValue)
            }),
            Expr::Paren(_) | Expr::TsAs(_) | Expr::TsTypeAssertion(_) => {
                infer_generic_constructor_expr_type(
                    expr,
                    self.interfaces,
                    self.generic_interfaces,
                    std::slice::from_ref(&self.scope),
                    &self.generic_call_returns,
                )
                .ok()
            }
            // A call chained directly off another call's return value
            // (`dayjs("2024-01-15").format(...)`, no `const` binding in
            // between) -- the receiver's real type isn't recovered from
            // `self.scope` the way a bound variable's is, since there's
            // no variable at all. Only the *type* is inspected here, via
            // the plain AST node -- the receiver itself is lowered and
            // evaluated exactly once, further down, when it's spliced
            // into the synthesized call (or, for the dynamic-`JsValue`
            // path, re-lowered through `lower_expr_with_expected_type`).
            Expr::Call(inner) => match &inner.callee {
                // The callee is an ordinary top-level function/ambient
                // declaration (`dayjs(...)`, `z.string()` after import
                // rewriting) -- looks up its own *declared* (not
                // per-call-site-substituted) return type directly.
                // Correct for a non-generic declaration; a genuinely
                // generic callee's `ret` here is still its own
                // unsubstituted placeholder shape, so those are excluded
                // -- *unless* that unsubstituted shape is already
                // `JsValue`, which is safe regardless of substitution:
                // `JsValue` means the declared return type (`ZodObject<T>`,
                // an unresolved interface reference) could never resolve
                // to anything JSON-representable no matter what `T`
                // becomes, since the problem is structural (the interface
                // itself is unknown), not `T`-dependent. Only a return
                // type that genuinely varies with `T` (`F64`/`Array(T)`/
                // etc., or `T` itself, e.g. a hypothetical `identity<T>(x:
                // T): T`) stays excluded. Real example: zod's own
                // `object<T extends ...>(shape: T): ZodObject<T>` --
                // without this, `z.object({...}).refine(...)` (no
                // intermediate `const` at all) failed outright
                // ("unsupported member call target"), even though the
                // exact same expression bound to an explicitly annotated
                // `const obj: JsValue = z.object({...})` already worked.
                Callee::Expr(callee) if matches!(callee.as_ref(), Expr::Ident(_)) => {
                    let Expr::Ident(identifier) = callee.as_ref() else {
                        unreachable!()
                    };
                    self.signatures
                        .get(identifier.sym.as_ref())
                        .filter(|signature| {
                            signature.generic_type_params.is_empty()
                                || signature.ret == HirType::JsValue
                        })
                        .map(|signature| signature.ret.clone())
                }
                // The callee is itself a member expression -- this call
                // is a chained method call (`z.string().min(2)`, itself
                // the receiver of a further `.max(10)`). A dynamic
                // method's real return type has no declared shape to
                // look up at all (see `lower_dynamic_value_method_call`'s
                // own doc comment) -- but by the same convention its
                // *runtime* dispatch already uses (a chained receiver
                // unconditionally needs a real handle), a method invoked
                // on a `JsValue` receiver is assumed to also yield
                // another `JsValue` for chaining purposes: real
                // zod/dayjs-style builder chains almost universally keep
                // returning "more of the same" object. Recurses into the
                // *inner* member's own object so an arbitrarily long
                // chain (`.a().b().c()`) resolves correctly at every
                // link.
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Member(inner_member) => self
                        .infer_member_receiver_type(&inner_member.obj)
                        .filter(|ty| *ty == HirType::JsValue),
                    _ => None,
                },
                Callee::Super(_) | Callee::Import(_) => None,
            },
            _ => None,
        }
    }

    fn lower_dynamic_value_method_call(
        &mut self,
        receiver_expr: &Expr,
        property: &str,
        args: &[swc_ecma_ast::ExprOrSpread],
        expected: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if args.iter().any(|argument| argument.spread.is_some()) {
            return Err(format!(
                "dynamic value method `{property}` does not support spread arguments"
            ));
        }
        let intrinsic = match (property, expected) {
            ("toString", _) => "callDynamicMethod",
            (_, Some(HirType::Dynamic)) => "callDynamicMethodHandleRaw",
            (_, Some(HirType::JsValue)) => "callDynamicMethodHandle",
            _ => "callDynamicMethod",
        };
        // The receiver must always come back as a genuine handle here,
        // never a JSON-decoded snapshot -- it's about to be fed straight
        // into `callDynamicMethod`/`callDynamicMethodHandle`, which needs
        // the real handle id, not a value. Unlike *this* call's own
        // `expected` (only `Some(JsValue)` when an outer `let`/`const`
        // annotation or a further chained call asked for it), the
        // receiver's need for a real handle is unconditional -- so this
        // is a second, always-on user of `lower_expr_with_expected_type`'s
        // one-shot hint (see its own doc comment), not a reuse of
        // `expected`. Real example: `z.string().min(2).max(10)` -- `.max`'s
        // receiver is itself the *method call* `z.string().min(2)`, which
        // needs this same hint recursively for its own receiver in turn.
        let receiver = self.lower_expr_with_expected_type(receiver_expr, Some(&HirType::JsValue))?;
        // Each argument gets the same one-shot `JsValue` hint the receiver
        // just did, for the identical reason: an argument that's itself a
        // method call chained off a `JsValue` receiver (`z.string().pipe(
        // z.string().min(3))`, real zod) would otherwise lower with no
        // hint at all, defaulting to the JSON-decoding snapshot behavior
        // (a content-free `{}`, the same failure mode `lower_object_lit_
        // field_value` was fixed for -- an object-literal field, not a
        // method-call argument, a different sink point for the identical
        // bug). A no-op for any argument that isn't itself a call
        // (`lower_expr_with_expected_type` only sets the hint for
        // `Expr::Call`, consumed and cleared the moment that call is
        // lowered), so this can't affect an ordinary literal/variable
        // argument.
        let json_args = args
            .iter()
            .map(|argument| {
                let value = match argument.expr.as_ref() {
                    Expr::Arrow(arrow) => {
                        let expected_return = if arrow.is_async {
                            HirType::Promise(Box::new(HirType::JsValue))
                        } else {
                            HirType::JsValue
                        };
                        self.lower_contextual_arrow(
                            arrow,
                            &vec![HirType::JsValue; arrow.params.len()],
                            Some(&expected_return),
                        )?
                    }
                    _ => self.lower_expr_with_expected_type(
                        &argument.expr,
                        Some(&HirType::JsValue),
                    )?,
                };
                self.coerce_to_declared(&HirType::Json, value)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let array = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(json_args))?;
        let call = HirExpr::Call(
            Box::new(HirExpr::Var(intrinsic.to_string())),
            vec![receiver, HirExpr::Lit(HirLit::Str(property.to_string())), array],
        );
        if property == "toString" {
            Ok(HirExpr::JsonAsString(Box::new(call)))
        } else {
            Ok(call)
        }
    }

    /// Lowers `receiver.property` (a plain property *read*, no call at
    /// all) where `receiver` (already lowered by the caller -- see its
    /// own doc comment for why re-lowering it here would risk double-
    /// evaluating a side-effecting receiver expression) is already known
    /// to be `HirType::JsValue`-typed. Reuses the existing
    /// `getDynamicProperty` intrinsic (`thaw_js_get_property_result`,
    /// thaw-quickjs), previously only a manual escape hatch nobody
    /// actually wired up to ordinary `.property` syntax, the same way
    /// `callDynamicMethod` was before it got wired up to `.method()`.
    /// Reads the property by plain lookup, not by enumeration --
    /// correctly finds a non-enumerable own property real zod's own
    /// `ZodError.issues` deliberately is (an *unannotated* method call's
    /// own default JSON-snapshot behavior, by contrast, would silently
    /// lose it, since a JSON encode can only ever capture enumerable
    /// properties).
    ///
    /// Unlike a method call, `getDynamicProperty` always hands back a
    /// real handle -- there's no JSON-decoding sibling to choose between
    /// based on an expected-type hint, so this doesn't need one either.
    /// A chained property read off *this* one (`bad.error.issues`)
    /// recurses back into this same lowering for free, since the result
    /// here is already `JsValue`-typed.
    fn lower_dynamic_value_property_read(
        &mut self,
        receiver: HirExpr,
        property: HirExpr,
    ) -> Result<HirExpr, String> {
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicProperty".to_string())),
            vec![receiver, property],
        ))
    }

    fn lower_primitive_conversion(
        &mut self,
        callee_name: &str,
        value: HirExpr,
        mut bindings: Vec<LoweredBinding>,
    ) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        let result = if callee_name == "String" && ty == HirType::Str {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_to_string".to_string())),
                vec![value],
            )
        } else if callee_name == "String" && ty == HirType::Bool {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                vec![value],
            )
        } else if callee_name == "String" && ty == HirType::F64 {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                vec![value],
            )
        } else if callee_name == "String"
            && matches!(
                ty,
                HirType::Array(_)
                    | HirType::Tuple(_)
                    | HirType::Object(_)
                    | HirType::Optional(_)
            )
        {
            self.coerce_primitive_to_string(value)?
        } else if callee_name == "Boolean" && ty != HirType::Json {
            let name = format!("__thaw_boolean_value_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            let converted = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
            bindings.push((name, ty, value));
            converted
        } else if callee_name == "Number" && ty == HirType::F64 {
            value
        } else if callee_name == "Number" && ty == HirType::Bool {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                vec![value],
            )
        } else if callee_name == "Number" && ty == HirType::Str {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                vec![value],
            )
        } else if callee_name == "Number" && ty == HirType::JsValue {
            HirExpr::JsonAsNumber(Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("readDynamicValue".to_string())),
                vec![value],
            )))
        } else if callee_name == "Number"
            && matches!(
                ty,
                HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
            )
        {
            self.coerce_primitive_to_number(value)?
        } else if ty != HirType::Json {
            return Err(format!(
                "`{callee_name}(...)` is only supported on a JSON value for now (got {ty:?})"
            ));
        } else {
            match callee_name {
                "Number" => HirExpr::JsonAsNumber(Box::new(value)),
                "String" => HirExpr::JsonAsString(Box::new(value)),
                _ => HirExpr::JsonAsBool(Box::new(value)),
            }
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }

}
