impl<'a> FnLowerer<'a> {
    fn class_to_json_method(&self, ty: &HirType) -> bool {
        class_name_from_type(ty).is_some_and(|class| {
            self.signatures
                .get(&class_method_symbol(class, "toJSON"))
                .is_some_and(|signature| signature.params.len() == 1)
        })
    }

    /// Whether `JSON.stringify` of a `ty` value must run a class `toJSON`
    /// somewhere inside it (the native encoder knows nothing of methods).
    fn type_needs_class_to_json(&self, ty: &HirType, seen: &mut Vec<String>) -> bool {
        if self.class_to_json_method(ty) {
            return true;
        }
        match ty {
            HirType::Array(element) => self.type_needs_class_to_json(element, seen),
            HirType::Object(fields) => {
                let class = class_name_from_type(ty);
                if let Some(class) = class {
                    if seen.iter().any(|name| name == class) {
                        return false;
                    }
                    seen.push(class.to_string());
                } else if fields.iter().any(|(_, field)| {
                    matches!(field, HirType::Function(..) | HirType::CallableFunction(..) | HirType::JsValue)
                }) {
                    // Function-valued fields keep the existing object-literal `toJSON` handling.
                    return false;
                }
                fields.iter().skip(usize::from(class.is_some())).any(|(name, field)| {
                    !name.starts_with(PRIVATE_SLOT_PREFIX)
                        && !is_hidden_accessor_field(name)
                        && self.type_needs_class_to_json(field, seen)
                })
            }
            _ => false,
        }
    }

    /// Replaces class instances that define `toJSON()` (directly, as array
    /// elements, or as fields of a class instance) with their `toJSON()`
    /// result so the native encoder sees what `JSON.stringify` would.
    fn apply_class_to_json(
        &mut self,
        value: HirExpr,
        ty: &HirType,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        if !self.type_needs_class_to_json(ty, &mut Vec::new()) {
            return Ok(None);
        }
        let span = swc_common::DUMMY_SP;
        let name = format!("__thaw_to_json_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), ty.clone());
        let ident = |name: &str| Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(name.into(), span));
        let method_call = |receiver: Expr| {
            Expr::Call(CallExpr {
                span,
                ctxt: Default::default(),
                callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                    span,
                    obj: Box::new(receiver),
                    prop: MemberProp::Ident(swc_ecma_ast::IdentName::new("toJSON".into(), span)),
                }))),
                args: Vec::new(),
                type_args: None,
            })
        };
        let result = if self.class_to_json_method(ty) {
            self.lower_expr(&method_call(ident(&name)))?
        } else if let HirType::Array(element) = ty {
            if !self.class_to_json_method(element) {
                return Ok(None);
            }
            let callback = Expr::Arrow(swc_ecma_ast::ArrowExpr {
                span,
                ctxt: Default::default(),
                params: vec![swc_ecma_ast::Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: swc_ecma_ast::Ident::new_no_ctxt("__thaw_to_json_item".into(), span),
                    type_ann: None,
                })],
                body: Box::new(swc_ecma_ast::ArrowFunctionBody::Expr(Box::new(method_call(
                    ident("__thaw_to_json_item"),
                )))),
                is_async: false,
                is_generator: false,
                type_params: None,
                return_type: None,
            });
            self.lower_expr(&Expr::Call(CallExpr {
                span,
                ctxt: Default::default(),
                callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                    span,
                    obj: Box::new(ident(&name)),
                    prop: MemberProp::Ident(swc_ecma_ast::IdentName::new("map".into(), span)),
                }))),
                args: vec![swc_ecma_ast::ExprOrSpread { spread: None, expr: Box::new(callback) }],
                type_args: None,
            }))?
        } else if let HirType::Object(fields) = ty {
            let mut snapshot = Vec::new();
            for (field, field_type) in fields.iter().skip(usize::from(class_name_from_type(ty).is_some())) {
                if field.starts_with(PRIVATE_SLOT_PREFIX)
                    || is_hidden_accessor_field(field)
                    || matches!(
                        field_type,
                        HirType::Function(..) | HirType::CallableFunction(..) | HirType::JsValue
                    )
                {
                    continue;
                }
                let read = self.lower_fixed_object_property_read(
                    HirExpr::Var(name.clone()),
                    fields,
                    field,
                )?;
                let read = match self.apply_class_to_json(read.clone(), field_type)? {
                    Some((converted, _)) => converted,
                    None => read,
                };
                snapshot.push((field.clone(), read));
            }
            HirExpr::ObjectLit(snapshot)
        } else {
            return Ok(None);
        };
        let result_type = self.infer_expr_type(&result)?;
        let wrapped = self.wrap_call_argument_bindings(result, &[(name, ty.clone(), value)])?;
        Ok(Some((wrapped, result_type)))
    }

    /// Native subclasses of `class_name` that declare their own instance
    /// `method` with the same signature, shallowest first. A call through a
    /// base-typed receiver must dispatch to these when the runtime identity
    /// says so, because method calls are otherwise bound statically.
    fn virtual_method_overrides(&self, class_name: &str, method: &str) -> Vec<(Symbol, HirType)> {
        let base_symbol = class_method_symbol(class_name, method);
        let Some(base) = self.signatures.get(&base_symbol) else {
            return Vec::new();
        };
        self.native_subclasses(class_name)
            .into_iter()
            .filter(|(name, _)| {
                self.signatures
                    .get(&class_method_symbol(name, method))
                    .is_some_and(|own| {
                        // A derived override may append (optional) parameters.
                        own.accessor_owner.as_deref() == Some(name.as_str())
                            && own.params.len() >= base.params.len()
                            && own.params[1..base.params.len()] == base.params[1..]
                            && own.ret == base.ret
                            && own.is_async == base.is_async
                            && own.native_rest == base.native_rest
                    })
            })
            .collect()
    }

    /// Native subclasses of `class_name` that redefine the instance accessor
    /// `property` (`setter` picks which half) with the same value type,
    /// shallowest first.
    fn virtual_accessor_overrides(
        &self,
        class_name: &str,
        property: &str,
        setter: bool,
    ) -> Vec<(Symbol, HirType)> {
        let symbol = |class: &str| {
            if setter {
                class_setter_symbol(class, property, false)
            } else {
                class_getter_symbol(class, property, false)
            }
        };
        let Some(base) = self.signatures.get(&symbol(class_name)) else {
            return Vec::new();
        };
        self.native_subclasses(class_name)
            .into_iter()
            .filter(|(name, _)| {
                self.signatures.get(&symbol(name)).is_some_and(|own| {
                    own.accessor_owner.as_deref() == Some(name.as_str())
                        && own.params.get(1..) == base.params.get(1..)
                        && own.ret == base.ret
                })
            })
            .collect()
    }

    /// Strict native subclasses of `class_name`, shallowest first.
    fn native_subclasses(&self, class_name: &str) -> Vec<(Symbol, HirType)> {
        let mut found = Vec::new();
        for (name, ty) in self.interfaces {
            let HirType::Object(fields) = ty else { continue };
            let Some(ancestry) = fields
                .first()
                .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
            else {
                continue;
            };
            let chain: Vec<&str> = ancestry.split('\u{1f}').collect();
            if chain.first() == Some(&name.as_str()) && chain[1..].contains(&class_name) {
                found.push((chain.len(), name.clone(), ty.clone()));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        found.into_iter().map(|(_, name, ty)| (name, ty)).collect()
    }

    /// Read of instance getter `owner.property` on `receiver` (a variable):
    /// dispatches to the most derived override by runtime class identity.
    fn call_class_instance_getter(
        &self,
        owner: &str,
        property: &str,
        receiver: HirExpr,
    ) -> HirExpr {
        let call = |class: &str, receiver: HirExpr| {
            HirExpr::Call(
                Box::new(HirExpr::Var(class_getter_symbol(class, property, false))),
                vec![self.assert_class_accessor_receiver(receiver, class)],
            )
        };
        let ret = self.signatures[&class_getter_symbol(owner, property, false)].ret.clone();
        let mut result = call(owner, receiver.clone());
        for (derived, derived_type) in self.virtual_accessor_overrides(owner, property, false) {
            let cast = HirExpr::TypedClosure(derived_type, Box::new(receiver.clone()));
            result = HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_has_class_identity".into())),
                    vec![receiver.clone(), HirExpr::Lit(HirLit::Str(derived.to_string()))],
                )),
                Box::new(call(&derived, cast)),
                Box::new(result),
                ret.clone(),
            );
        }
        result
    }

    /// `recv.m(...)` where a subclass overrides `m`: test the runtime class
    /// identity (most derived first) and call that class's implementation.
    fn lower_virtual_class_method_call(
        &mut self,
        call: &CallExpr,
        member: &MemberExpr,
        receiver_type: &HirType,
        property: &str,
        overrides: &[(Symbol, HirType)],
        native_forward: bool,
    ) -> Result<HirExpr, String> {
        let class_name = class_name_from_type(receiver_type)
            .expect("virtual dispatch receivers are native classes")
            .to_string();
        let receiver = self.lower_expr(&member.obj)?;
        let receiver_name = format!("__thaw_virtual_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), receiver_type.clone());
        let build_call = |this: &mut Self, class: &str, receiver: &str| {
            let mut args = vec![swc_ecma_ast::ExprOrSpread {
                spread: None,
                expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    receiver.into(),
                    call.span,
                ))),
            }];
            args.extend(call.args.iter().cloned());
            this.lower_call_with_class_guard(
                &CallExpr {
                    span: call.span,
                    ctxt: call.ctxt,
                    callee: Callee::Expr(Box::new(Expr::Ident(
                        swc_ecma_ast::Ident::new_no_ctxt(
                            class_method_symbol(class, property).into(),
                            call.span,
                        ),
                    ))),
                    args,
                    type_args: None,
                },
                Some(class),
            )
        };
        // `native_forward`: the receiver's class does not define the method; the base
        // behaviour is the wrapped `Map`/`Set`/`Array` method.
        let mut result = if native_forward {
            let receiver_ident = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                receiver_name.as_str().into(),
                call.span,
            ));
            let mut forwarded = call.clone();
            if let Callee::Expr(callee) = &mut forwarded.callee {
                if let Expr::Member(forwarded_member) = callee.as_mut() {
                    *forwarded_member.obj = native_base_member(receiver_ident.clone());
                }
            }
            self.lower_native_base_forward(&forwarded, property, &receiver_ident)?
        } else {
            build_call(self, &class_name, &receiver_name)?
        };
        let result_type = self.infer_expr_type(&result)?;
        for (derived, derived_type) in overrides {
            let derived_name = format!("__thaw_virtual_receiver_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(derived_name.clone(), derived_type.clone());
            let invoked = build_call(self, derived, &derived_name)?;
            let invoked = if self.infer_expr_type(&invoked)? == result_type {
                invoked
            } else {
                HirExpr::TypedClosure(result_type.clone(), Box::new(invoked))
            };
            let invoked = self.wrap_call_argument_bindings(
                invoked,
                &[(
                    derived_name,
                    derived_type.clone(),
                    HirExpr::TypedClosure(
                        derived_type.clone(),
                        Box::new(HirExpr::Var(receiver_name.clone())),
                    ),
                )],
            )?;
            result = HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_has_class_identity".into())),
                    vec![
                        HirExpr::Var(receiver_name.clone()),
                        HirExpr::Lit(HirLit::Str(derived.to_string())),
                    ],
                )),
                Box::new(invoked),
                Box::new(result),
                result_type.clone(),
            );
        }
        self.wrap_call_argument_bindings(
            result,
            &[(receiver_name, receiver_type.clone(), receiver)],
        )
    }

    fn is_ordinary_quickjs_typed_call(backend: DynamicBackend, symbol: &str) -> bool {
        is_ordinary_quickjs_symbol(backend, symbol)
    }

    fn is_quickjs_instance_typed_method(backend: DynamicBackend, symbol: &str) -> bool {
        backend == DynamicBackend::QuickJs
            && (symbol.starts_with("$method$") || symbol.starts_with("$methodvoid$"))
    }

    fn is_napi_constructor_typed_call(
        backend: DynamicBackend,
        symbol: &str,
        ret: &HirType,
    ) -> bool {
        backend == DynamicBackend::Napi && *ret == HirType::JsValue && symbol.starts_with("$new$")
    }

    fn contains_live_host_jsvalue_json_conversion(expr: &HirExpr) -> bool {
        match expr {
            HirExpr::JsValueAsJson(_) => true,
            HirExpr::Conditional(_, yes, no, _) => {
                Self::contains_live_host_jsvalue_json_conversion(yes)
                    || Self::contains_live_host_jsvalue_json_conversion(no)
            }
            HirExpr::Call(callee, _) => match callee.as_ref() {
                HirExpr::Lambda(_, _, _, body) => {
                    Self::contains_live_host_jsvalue_json_conversion(body)
                }
                _ => false,
            },
            HirExpr::Block(statements) => statements.iter().any(|statement| match statement {
                HirStmt::Expr(value) | HirStmt::Return(Some(value)) => {
                    Self::contains_live_host_jsvalue_json_conversion(value)
                }
                _ => false,
            }),
            _ => false,
        }
    }

    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        // `o["name"](...)` is `o.name(...)`: canonicalize identifier-like
        // string keys so every member-call path (dynamic receivers, nested
        // receivers, optional chains) handles them.
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::Member(member) = callee.as_ref() {
                if let MemberProp::Computed(computed) = &member.prop {
                    if let Expr::Lit(swc_ecma_ast::Lit::Str(key)) = computed.expr.as_ref() {
                        let key = key.value.to_string_lossy();
                        let mut chars = key.chars();
                        let identifier_like = chars
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
                            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
                        if identifier_like {
                            let mut canonical = call.clone();
                            if let Callee::Expr(callee) = &mut canonical.callee {
                                if let Expr::Member(member) = callee.as_mut() {
                                    member.prop = MemberProp::Ident(swc_ecma_ast::IdentName::new(
                                        key.as_ref().into(),
                                        computed.span,
                                    ));
                                }
                            }
                            return self.lower_call(&canonical);
                        }
                    }
                }
            }
        }
        if let Callee::Expr(callee) = &call.callee {
            if matches!(callee.as_ref(), Expr::Ident(class) if class.sym == *"Array")
                && !self.scope.contains_key("Array")
            {
                return self.lower_array_constructor(&call.args, call.type_args.as_deref(), None);
            }
        }
        if let Some(receiver) = self.array_constructor_fill_receiver(call) {
            let mut typed = call.clone();
            if let Callee::Expr(callee) = &mut typed.callee {
                if let Expr::Member(member) = callee.as_mut() {
                    *member.obj = receiver;
                }
            }
            return self.lower_call_with_class_guard_and_target(&typed, None, None);
        }
        self.lower_call_with_class_guard_and_target(call, None, None)
    }

    /// `super(...)` of a class extending `Map`/`Set`: build the wrapped native
    /// collection exactly as `new Map<K, V>(...)` would and store it in the
    /// hidden base field.
    fn lower_native_collection_super(
        &mut self,
        base_name: &str,
        args: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<HirExpr, String> {
        let this_name = self.resolve_binding("this");
        let this_type = self
            .scope
            .get(&this_name)
            .cloned()
            .ok_or("`super(...)` used outside a constructor")?;
        let value = if base_name == "Array" {
            let Some(array_type) = native_base_type(&this_type).cloned() else {
                return Err("`extends Array` class has no array layout".into());
            };
            let HirType::Array(element) = array_type.clone() else {
                return Err("`extends Array` class has no array layout".into());
            };
            let value = if args.is_empty() {
                HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), *element)
            } else if args.iter().any(|argument| argument.spread.is_some()) {
                // `super(...xs)`: the elements, copied (a lone number is not a length here).
                let literal = Expr::Array(swc_ecma_ast::ArrayLit {
                    span: swc_common::DUMMY_SP,
                    elems: args.iter().cloned().map(Some).collect(),
                });
                self.lower_expr_with_expected_type(&literal, Some(&array_type))?
            } else {
                self.lower_array_constructor(args, self.native_base_type_args.clone().as_deref(), None)?
            };
            self.coerce_to_declared(&array_type, value)?
        } else if let (Some(argument), true) = (
            args.first(),
            args.len() == 1
                && args[0].spread.is_none()
                && matches!(base_name, "Map" | "Set")
                && self.native_super_dispatches_overrides(base_name, &this_type),
        ) {
            // `new Map(iterable)` calls `this.set(key, value)` per entry (`add` for a `Set`),
            // so an overriding subclass sees every entry.
            return self.lower_native_collection_super_by_method(base_name, argument);
        } else {
            let construct = Expr::New(swc_ecma_ast::NewExpr {
                span: swc_common::DUMMY_SP,
                ctxt: Default::default(),
                callee: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    base_name.into(),
                    swc_common::DUMMY_SP,
                ))),
                type_args: self.native_base_type_args.clone(),
                args: Some(args.to_vec()),
            });
            self.lower_expr(&construct)?
        };
        Ok(HirExpr::PropAssign(
            Box::new(HirExpr::Var(this_name)),
            this_type,
            NATIVE_BASE_FIELD.to_string(),
            Box::new(value),
        ))
    }

    /// Whether `set` (`Map`) / `add` (`Set`) is defined by a user class that can be the dynamic
    /// class of an instance built from `this_type`: its own or an inherited class, or a subclass.
    fn native_super_dispatches_overrides(&self, base_name: &str, this_type: &HirType) -> bool {
        let method = if base_name == "Map" { "set" } else { "add" };
        let defines = |class: &str| {
            self.signatures
                .get(&class_method_symbol(class, method))
                .is_some_and(|own| own.accessor_owner.as_deref() == Some(class))
        };
        let Some(class) = class_name_from_type(this_type) else {
            return false;
        };
        let chain = match this_type {
            HirType::Object(fields) => fields
                .first()
                .and_then(|(marker, _)| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
                .map(|chain| chain.split('\u{1f}').collect::<Vec<_>>())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        chain.into_iter().any(defines)
            || self
                .native_subclasses(class)
                .iter()
                .any(|(name, _)| defines(name))
    }

    /// `super(iterable)` of a `Map`/`Set` subclass overriding `set`/`add`: an empty collection,
    /// then one call of the (dynamically dispatched) method per entry.
    fn lower_native_collection_super_by_method(
        &mut self,
        base_name: &str,
        argument: &swc_ecma_ast::ExprOrSpread,
    ) -> Result<HirExpr, String> {
        let this_name = self.resolve_binding("this");
        let empty = self.lower_native_collection_super(base_name, &[])?;
        let source = self.lower_expr(&argument.expr)?;
        let source = self.native_collection_entries(source)?;
        let source_type = self.infer_expr_type(&source)?;
        let ident = |name: &str| {
            Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(name.into(), swc_common::DUMMY_SP))
        };
        let declare = |this: &mut Self, name: &str, ty: &HirType| {
            this.scope.insert(name.to_string(), ty.clone());
            this.bindings.entry(name.to_string()).or_default().push(name.to_string());
        };
        let id = self.next_binding;
        self.next_binding += 1;
        let source_name = format!("__thaw_super_source_{id}");
        let (payload_name, payload_type, absent) = match &source_type {
            HirType::Optional(payload) => (
                format!("__thaw_super_entries_{id}"),
                payload.as_ref().clone(),
                Some(HirExpr::OptionalIsNone(
                    Box::new(HirExpr::Var(source_name.clone())),
                    payload.as_ref().clone(),
                )),
            ),
            other => (source_name.clone(), other.clone(), None),
        };
        declare(self, &source_name, &source_type);
        if absent.is_some() {
            declare(self, &payload_name, &payload_type);
        }
        let call_method = |method: &str, arguments: Vec<Expr>| {
            Stmt::Expr(swc_ecma_ast::ExprStmt {
                span: swc_common::DUMMY_SP,
                expr: Box::new(Expr::Call(CallExpr {
                    span: swc_common::DUMMY_SP,
                    ctxt: Default::default(),
                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                        span: swc_common::DUMMY_SP,
                        obj: Box::new(Expr::This(swc_ecma_ast::ThisExpr {
                            span: swc_common::DUMMY_SP,
                        })),
                        prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                            method.into(),
                            swc_common::DUMMY_SP,
                        )),
                    }))),
                    args: arguments
                        .into_iter()
                        .map(|expr| swc_ecma_ast::ExprOrSpread { spread: None, expr: Box::new(expr) })
                        .collect(),
                    type_args: None,
                })),
            })
        };
        let entry = format!("__thaw_super_entry_{id}");
        let index = |position: f64| {
            Expr::Member(MemberExpr {
                span: swc_common::DUMMY_SP,
                obj: Box::new(ident(&entry)),
                prop: MemberProp::Computed(swc_ecma_ast::ComputedPropName {
                    span: swc_common::DUMMY_SP,
                    expr: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                        span: swc_common::DUMMY_SP,
                        value: position,
                        raw: None,
                    }))),
                }),
            })
        };
        let body = if base_name == "Map" {
            call_method("set", vec![index(0.0), index(1.0)])
        } else {
            call_method("add", vec![ident(&entry)])
        };
        let looped = Stmt::ForOf(swc_ecma_ast::ForOfStmt {
            span: swc_common::DUMMY_SP,
            is_await: false,
            left: swc_ecma_ast::ForHead::VarDecl(Box::new(swc_ecma_ast::VarDecl {
                span: swc_common::DUMMY_SP,
                ctxt: Default::default(),
                kind: swc_ecma_ast::VarDeclKind::Const,
                declare: false,
                decls: vec![swc_ecma_ast::VarDeclarator {
                    span: swc_common::DUMMY_SP,
                    name: Pat::Ident(swc_ecma_ast::BindingIdent {
                        id: swc_ecma_ast::Ident::new_no_ctxt(entry.as_str().into(), swc_common::DUMMY_SP),
                        type_ann: None,
                    }),
                    init: None,
                    definite: false,
                }],
            })),
            right: Box::new(ident(&payload_name)),
            body: Box::new(Stmt::Block(swc_ecma_ast::BlockStmt {
                span: swc_common::DUMMY_SP,
                ctxt: Default::default(),
                stmts: vec![body],
            })),
        });
        let iteration = self.lower_stmt_seq(&looped)?;
        let mut statements = vec![
            HirStmt::Expr(empty),
            HirStmt::Let(source_name.clone(), source_type.clone(), source),
        ];
        match absent {
            Some(absent) => statements.push(HirStmt::If(
                absent,
                Vec::new(),
                std::iter::once(HirStmt::Let(
                    payload_name,
                    payload_type.clone(),
                    HirExpr::OptionalValue(Box::new(HirExpr::Var(source_name)), payload_type),
                ))
                .chain(iteration)
                .collect(),
            )),
            None => statements.extend(iteration),
        }
        statements.push(HirStmt::Return(Some(HirExpr::Var(this_name))));
        Ok(HirExpr::Block(statements))
    }

    /// A `Map`/`Set` subclass instance used where the native collection is
    /// consumed (spread, `for...of`, conversions): its wrapped collection.
    fn project_native_base(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let ty = self.infer_expr_type(&value)?;
        if native_base_type(&ty).is_none() {
            return Ok(value);
        }
        Ok(HirExpr::PropAccess(
            Box::new(value),
            ty,
            NATIVE_BASE_FIELD.to_string(),
        ))
    }

    /// Whether `expr` is a call chain (`stack.filter(..)`, `Stack.from(..)`, ...) rooted at an
    /// `extends Array` class whose methods yield instances of that class.
    fn is_array_subclass_chain(&self, expr: &Expr) -> bool {
        let Expr::Call(call) = peel_parens(expr) else {
            return false;
        };
        let Callee::Expr(callee) = &call.callee else {
            return false;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return false;
        };
        let Some(method) = member_property_name(&member.prop) else {
            return false;
        };
        let array_class = |ty: &HirType| matches!(native_base_type(ty), Some(HirType::Array(_)));
        if let Expr::Ident(class) = member.obj.as_ref() {
            if matches!(method.as_str(), "from" | "of") {
                return self.interfaces.get(class.sym.as_ref()).is_some_and(array_class);
            }
        }
        if !matches!(
            method.as_str(),
            "map" | "filter" | "slice" | "splice" | "concat" | "flat" | "flatMap" | "sort" | "reverse" | "fill" | "copyWithin"
        ) {
            return false;
        }
        match self.infer_member_receiver_type(&member.obj) {
            Some(ty) => array_class(&ty),
            None => self.is_array_subclass_chain(&member.obj),
        }
    }

    /// The class names an `instanceof C` target stands for: `C` itself plus, for a generic
    /// class, every instantiation it was monomorphised into (`C__thaw_f64`, ...).
    pub(in crate::lower) fn instanceof_class_names(&self, class: &str) -> Vec<String> {
        let prefix = format!("{class}__thaw_");
        let mut names = vec![class.to_string()];
        let mut instances = self
            .interfaces
            .keys()
            .filter(|name| name.starts_with(&prefix))
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        instances.sort();
        names.extend(instances);
        names
    }

    /// A `Map`/`Set` consumed as an iterable (`new Map(other)`, `const [first] = set`):
    /// the snapshot array of its entries / values, as `[...collection]` builds it.
    pub(in crate::lower) fn native_collection_entries(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let value = self.project_native_base(value)?;
        let (intrinsic, array_type) = match self.infer_expr_type(&value)? {
            HirType::Map(key, element) => (
                "__thaw_map_snapshot_entries",
                HirType::Array(Box::new(HirType::Tuple(vec![*key, *element]))),
            ),
            HirType::Set(element) => ("__thaw_map_snapshot_keys", HirType::Array(element)),
            _ => return Ok(value),
        };
        Ok(HirExpr::TypedClosure(
            array_type,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var(intrinsic.into())), vec![value])),
        ))
    }

    /// `Symbol.species`: `stack.map(..)`, `filter`, `slice`, `splice`, `concat`, `flat`,
    /// `flatMap` (and `Stack.from`/`of`) of an `extends Array` class give another instance
    /// of that class, built by its constructor with the length Node passes
    /// (`new this.constructor(length)`). Representable when the class can be built from no
    /// arguments, from its rest items, or from one numeric argument. A result whose element
    /// type differs from the wrapped array's (`Stack<number>.map(x => "s")`) is the same
    /// class record retyped to carry the new element type; other constructors keep the
    /// plain array.
    fn wrap_native_species(
        &mut self,
        class_type: &HirType,
        result: HirExpr,
        method: &str,
    ) -> Result<HirExpr, String> {
        let Some(base @ HirType::Array(_)) = native_base_type(class_type) else {
            return Ok(result);
        };
        let base = base.clone();
        let result_type = self.infer_expr_type(&result)?;
        if !matches!(result_type, HirType::Array(_)) {
            return Ok(result);
        }
        let Some(class) = class_name_from_type(class_type) else {
            return Ok(result);
        };
        let constructor = class_constructor_symbol(class);
        let Some(signature) = self.signatures.get(&constructor) else {
            return Ok(result);
        };
        let params = signature.params.clone();
        let has_rest = signature.native_rest.is_some();
        let numeric = |ty: &HirType| match ty {
            HirType::F64 | HirType::Json => true,
            HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
                matches!(inner.as_ref(), HirType::F64 | HirType::Json)
            }
            HirType::Union(members) => members.contains(&HirType::F64),
            _ => false,
        };
        let result_name = format!("__thaw_species_items_{}", self.next_binding);
        self.next_binding += 1;
        let arguments = match params.as_slice() {
            [] => Vec::new(),
            [HirType::Array(items)] if has_rest => vec![HirExpr::ArrayAlloc(
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                items.as_ref().clone(),
            )],
            [length, rest @ ..]
                if numeric(length)
                    && rest.iter().all(|param| matches!(param, HirType::Optional(_) | HirType::Json)) =>
            {
                let count = if matches!(method, "map" | "slice" | "splice" | "of" | "from") {
                    HirExpr::ArrayLen(Box::new(HirExpr::Var(result_name.clone())))
                } else {
                    HirExpr::Lit(HirLit::F64(0.0))
                };
                // `Array.from(iterable)` calls `new C()` with no argument at all.
                let first = match length {
                    HirType::Optional(payload) if method == "from" => HirExpr::OptionalNone(payload.as_ref().clone()),
                    HirType::Json if method == "from" => self.coerce_to_declared(length, HirExpr::Lit(HirLit::Undefined))?,
                    _ => self.coerce_to_declared(length, count)?,
                };
                let mut arguments = vec![first];
                for param in rest {
                    arguments.push(match param {
                        HirType::Optional(payload) => HirExpr::OptionalNone(payload.as_ref().clone()),
                        // A required parameter widened to `any`: Node passes `undefined`.
                        _ => self.coerce_to_declared(param, HirExpr::Lit(HirLit::Undefined))?,
                    });
                }
                arguments
            }
            _ => return Ok(result),
        };
        let target_type = if result_type == base {
            class_type.clone()
        } else {
            let HirType::Object(fields) = class_type else {
                return Ok(result);
            };
            let mut fields = fields.clone();
            fields[1].1 = result_type.clone();
            HirType::Object(fields)
        };
        let name = format!("__thaw_species_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(name.clone(), class_type.clone());
        Ok(HirExpr::Block(vec![
            HirStmt::Let(result_name.clone(), result_type, result),
            HirStmt::Let(
                name.clone(),
                class_type.clone(),
                HirExpr::Call(Box::new(HirExpr::Var(constructor)), arguments),
            ),
            HirStmt::Expr(HirExpr::PropAssign(
                Box::new(HirExpr::Var(name.clone())),
                class_type.clone(),
                NATIVE_BASE_FIELD.to_string(),
                Box::new(HirExpr::TypedClosure(base, Box::new(HirExpr::Var(result_name)))),
            )),
            HirStmt::Return(Some(HirExpr::TypedClosure(
                target_type,
                Box::new(HirExpr::Var(name)),
            ))),
        ]))
    }

    /// Lowers a call forwarded to the wrapped collection. Chainable `set`/`add`
    /// return the subclass instance itself, as in JavaScript.
    fn lower_native_base_forward(
        &mut self,
        forwarded: &CallExpr,
        method: &str,
        receiver: &Expr,
    ) -> Result<HirExpr, String> {
        let result = self.lower_call(forwarded)?;
        if matches!(method, "map" | "filter" | "slice" | "splice" | "concat" | "flat" | "flatMap") {
            if let Some(receiver_type) = self.infer_member_receiver_type(receiver) {
                return self.wrap_native_species(&receiver_type, result, method);
            }
        }
        if matches!(method, "set" | "add" | "sort" | "reverse" | "fill" | "copyWithin")
            && matches!(receiver, Expr::Ident(_) | Expr::This(_))
        {
            let wrapped = self.infer_member_receiver_type(receiver);
            if wrapped.as_ref().and_then(native_base_type) == Some(&self.infer_expr_type(&result)?) {
                let this = self.lower_expr(receiver)?;
                return Ok(HirExpr::EvalThen(Box::new(result), Box::new(this)));
            }
        }
        Ok(result)
    }

    fn lower_generic_named_call(
        &mut self,
        call: &CallExpr,
        target: &str,
        expected_source_return_hint: Option<&HirType>,
        expected_return_hint: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let previous_source = std::mem::replace(
            &mut self.expected_source_return_hint,
            expected_source_return_hint.cloned(),
        );
        let previous = std::mem::replace(&mut self.expected_return_hint, expected_return_hint.cloned());
        let result = self.lower_call_with_class_guard_and_target(call, None, Some(target));
        self.expected_source_return_hint = previous_source;
        self.expected_return_hint = previous;
        result
    }

    /// The guard is set only by the source-proven `receiver.method(...)`
    /// rewrite below. Keep named-call overload/default/rest normalization in
    /// this shared path, then check the allocated class identity after all
    /// supplied arguments and their conversions have been evaluated.
    fn lower_call_with_class_guard(
        &mut self,
        call: &CallExpr,
        class_method_guard: Option<&str>,
    ) -> Result<HirExpr, String> {
        self.lower_call_with_class_guard_and_target(call, class_method_guard, None)
    }

    fn lower_call_with_class_guard_and_target(
        &mut self,
        call: &CallExpr,
        class_method_guard: Option<&str>,
        canonical_target: Option<&str>,
    ) -> Result<HirExpr, String> {
        // Taken (not just read) as the very first action, before this
        // call's own arguments are lowered below -- a hint set for *this*
        // call must never still be visible if lowering one of its own
        // arguments recurses into another `lower_call` for a nested call
        // expression, which would otherwise see and wrongly consume a
        // hint meant for its parent. See `lower_expr_with_expected_type`.
        let expected_return_hint = self.expected_return_hint.take();
        let expected_source_return_hint = self.expected_source_return_hint.take();
        if matches!(&call.callee, Callee::Expr(callee)
            if matches!(callee.as_ref(), Expr::Ident(name) if name.sym == "@@thaw_class_key_reference_error"))
        {
            let [message] = call.args.as_slice() else {
                return Err("internal class key error needs one message".into());
            };
            if message.spread.is_some() {
                return Err("internal class key error cannot spread its message".into());
            }
            let message = self.lower_expr(&message.expr)?;
            self.expect_type(&HirType::Str, &message, "class key error message")?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_frame".into())),
                vec![HirExpr::Lit(HirLit::Str("ReferenceError".into())), message],
            ));
        }
        // `process.hrtime.bigint()`: a BigInt cannot cross the JSON boundary of a
        // dynamic `process` call, so rebuild the exact value from `hrtime()`'s
        // `[seconds, nanoseconds]` pair as a native bigint.
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::Member(bigint) = callee.as_ref() {
                if let (Expr::Member(hrtime), MemberProp::Ident(property)) = (bigint.obj.as_ref(), &bigint.prop) {
                    if let (Expr::Ident(process), MemberProp::Ident(method)) = (hrtime.obj.as_ref(), &hrtime.prop) {
                        if property.sym == "bigint"
                            && method.sym == "hrtime"
                            && process.sym == "process"
                            && call.args.is_empty()
                            && !self.scope.contains_key(&self.resolve_binding("process"))
                        {
                            return self.lower_expr(&hrtime_bigint_expression(process.sym.as_ref()));
                        }
                    }
                }
            }
        }
        if matches!(
            &call.callee,
            Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Ident(ident)
                    if ident.sym == "__thaw_mark_typed_wrapper_json_argument")
        ) {
            return Err("typed-wrapper JSON ownership marker is compiler-private".into());
        }
        // `Stack.from(..)` / `Stack.of(..)` / `Stack.isArray(..)` / `Counter.groupBy(..)`: statics
        // inherited from `Array` / `Map` (through any number of subclasses). `from`/`of` produce
        // an instance of the class (see `wrap_native_species`); `Map.groupBy` is a plain `Map`.
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::Member(member) = callee.as_ref() {
                if let (Expr::Ident(class), MemberProp::Ident(property)) =
                    (member.obj.as_ref(), &member.prop)
                {
                    let class_type = self.interfaces.get(class.sym.as_ref()).cloned();
                    let builtin = match (
                        class_type.as_ref().and_then(native_base_type),
                        property.sym.as_ref(),
                    ) {
                        (Some(HirType::Array(_)), "from" | "of" | "isArray") => Some("Array"),
                        (Some(HirType::Map(..)), "groupBy") => Some("Map"),
                        _ => None,
                    };
                    if let Some(builtin) = builtin.filter(|_| {
                        self.is_unshadowed_class_identifier(class.sym.as_ref())
                            && !self.signatures.contains_key(&class_static_method_symbol(
                                class.sym.as_ref(),
                                property.sym.as_ref(),
                            ))
                    }) {
                        let mut forwarded = call.clone();
                        if let Callee::Expr(callee) = &mut forwarded.callee {
                            if let Expr::Member(forwarded_member) = callee.as_mut() {
                                *forwarded_member.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                    builtin.into(),
                                    class.span,
                                ));
                            }
                        }
                        let result = self.lower_call(&forwarded)?;
                        return match class_type {
                            Some(class_type) if matches!(property.sym.as_ref(), "from" | "of") => {
                                self.wrap_native_species(&class_type, result, property.sym.as_ref())
                            }
                            _ => Ok(result),
                        };
                    }
                }
            }
        }
        if matches!(call.callee, Callee::Super(_)) {
            let (symbol, _base_type, base_name) = self
                .super_initializer
                .clone()
                .ok_or("`super(...)` is only valid in a derived class constructor")?;
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`), so there is no base initializer
            // function to call -- `super(message)` for a class directly
            // extending one of them instead assigns the inherited
            // `message` field itself. A class extending *that* class still
            // goes through the normal call-the-base-initializer path below
            // (its own generated initializer function already runs this
            // assignment as part of its body), so only a *direct* `extends
            // Error`-family base needs this.
            if matches!(base_name.as_str(), "Map" | "Set" | "Array" | "Date" | "RegExp") && !self.signatures.contains_key(&symbol) {
                return self.lower_native_collection_super(&base_name, &call.args);
            }
            if matches!(
                base_name.as_str(),
                "Error"
                    | "TypeError"
                    | "RangeError"
                    | "SyntaxError"
                    | "ReferenceError"
                    | "EvalError"
                    | "URIError"
                    | "AggregateError"
                    | "SuppressedError"
            ) {
                if call.type_args.is_some() {
                    return Err("native `super(...)` does not support type arguments".into());
                }
                let [message] = call.args.as_slice() else {
                    return Err(format!(
                        "`super(...)` for `{base_name}` expects a message argument"
                    ));
                };
                if message.spread.is_some() {
                    return Err("`super(...)` does not support a spread argument".into());
                }
                let message = self.lower_expr(&message.expr)?;
                let message = self.coerce_primitive_to_string(message)?;
                let this_name = self.resolve_binding("this");
                let this_type = self
                    .scope
                    .get(&this_name)
                    .cloned()
                    .ok_or("`super(...)` used outside a constructor")?;
                let assign_message = HirExpr::PropAssign(
                    Box::new(HirExpr::Var(this_name.clone())),
                    this_type.clone(),
                    "message".to_string(),
                    Box::new(message),
                );
                // Defaults `.name` to the immediate native ancestor's own
                // name (e.g. `TypeError`), matching real JavaScript's
                // inherited `Error.prototype.name`/etc. -- see
                // `lower/module/classes.rs`'s field-layout comment. A
                // constructor that goes on to assign `this.name = "..."`
                // itself just overwrites this same real field afterwards.
                let assign_name = HirExpr::PropAssign(
                    Box::new(HirExpr::Var(this_name)),
                    this_type,
                    "name".to_string(),
                    Box::new(HirExpr::Lit(HirLit::Str(base_name.clone()))),
                );
                // A bare `HirExpr::Block` isn't itself a compilable
                // expression (only a lambda/function *body* is) --
                // wrapped as an immediately-invoked, paramless lambda,
                // the same "IIFE" shape `wrap_call_argument_bindings`
                // already produces elsewhere for sequencing more than
                // one effect out of a single expression-returning
                // call-lowering function. Every outer variable the body
                // references (`this`, and whatever `message` itself
                // reads) must be an explicit capture -- a lambda has no
                // implicit access to its enclosing scope.
                let body = HirExpr::Block(vec![
                    HirStmt::Expr(assign_message),
                    HirStmt::Expr(assign_name),
                ]);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter_map(|referenced| {
                        self.scope.get(&referenced).cloned().map(|ty| HirParam {
                            name: referenced,
                            ty,
                        })
                    })
                    .collect();
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        Vec::new(),
                        HirType::Void,
                        Box::new(body),
                    )),
                    Vec::new(),
                ));
            }
            let signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .ok_or_else(|| format!("missing base class initializer `{symbol}`"))?;
            if call.type_args.is_some() {
                return Err("native `super(...)` does not support type arguments".into());
            }
            let this_name = self.resolve_binding("this");
            let mut args = vec![HirExpr::Var(this_name)];
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "base constructor")?;
            let arguments = self.pad_source_mask_arguments(
                &signature,
                arguments,
                1,
                "base constructor",
            )?;
            args.extend(arguments);
            let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
            return self.wrap_call_argument_bindings(result, &bindings);
        }
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::SuperProp(member) = callee.as_ref() {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` member access is only valid in a derived class")?;
                let method_name = match &member.prop {
                    SuperProp::Ident(name) => name.sym.to_string(),
                    SuperProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                        _ => {
                            return Err(
                                "computed super methods require a string literal name".into()
                            );
                        }
                    },
                };
                let symbol = if self.class_static_context {
                    class_static_method_symbol(&base_name, &method_name)
                } else {
                    class_method_symbol(&base_name, &method_name)
                };
                // `super.set(...)` in a class extending `Map`/`Set`: call the
                // method on the wrapped native collection.
                if !self.class_static_context && !self.signatures.contains_key(&symbol) {
                    let this = self.resolve_binding("this");
                    if self.scope.get(&this).and_then(native_base_type).is_some() {
                        let mut forwarded = call.clone();
                        forwarded.callee = Callee::Expr(Box::new(Expr::Member(MemberExpr {
                            span: member.span,
                            obj: Box::new(native_base_member(Expr::This(swc_ecma_ast::ThisExpr {
                                span: member.span,
                            }))),
                            prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                                method_name.as_str().into(),
                                member.span,
                            )),
                        })));
                        let this = Expr::This(swc_ecma_ast::ThisExpr { span: member.span });
                        return self.lower_native_base_forward(&forwarded, &method_name, &this);
                    }
                }
                let signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no method `{method_name}`")
                })?;
                if call.type_args.is_some() {
                    return Err("native super methods do not support type arguments".into());
                }
                let receiver_count = usize::from(!self.class_static_context);
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                let label = format!("super method `{base_name}.{method_name}`");
                let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                let arguments = self.pad_source_mask_arguments(
                    &signature,
                    arguments,
                    receiver_count,
                    &label,
                )?;
                args.extend(arguments);
                let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        if let Some(invoked) = self.lower_saved_native_method_call_or_apply(call)? {
            return Ok(invoked);
        }
        if self.unbound_this_context {
            if let Expr::Member(member) = callee_expr.as_ref() {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` method call requires a statically known property")?;
                    let HirType::Function(_, result) = self
                        .unbound_this_member_type(&property)
                        .ok_or_else(|| format!("class has no native method `{property}`"))?
                    else {
                        return Err(format!("native member `{property}` is not callable"));
                    };
                    return self.lower_unbound_this_error(&property, &result);
                }
                // `this.field.method()` in a detached method reads a field of
                // `undefined`: unsupported, so the caller emits the throwing
                // stub instead of a mistyped dynamic call.
                let mut root = member.obj.as_ref();
                while let Expr::Member(inner) = root {
                    root = inner.obj.as_ref();
                }
                if matches!(root, Expr::This(_)) {
                    return Err("detached method call through a `this` field".into());
                }
            }
        }

        if let Some(invoked) = self.lower_immediately_invoked_class_bind(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_immediately_invoked_function_bind(call)? {
            return Ok(invoked);
        }

        // `Temporal.<Namespace>.<method>(...)` / `Temporal.Now.<method>()`.
        // Checked before the class/static-method resolution below, which
        // would otherwise try to resolve the `Temporal` namespace as a
        // variable.
        if let Expr::Member(member) = callee_expr.as_ref() {
            if let Some(result) = self.lower_temporal_namespace_call(member, call)? {
                return Ok(result);
            }
        }

        // Expressions that produce typed functions are callable through the
        // same closure ABI as locals. In particular, support `make(x)(y)`.
        // `fns[i](x)`: an element of a typed function array is a plain function
        // value, not a method, so it goes through the same closure ABI.
        let indexed_function_element = matches!(callee_expr.as_ref(), Expr::Member(member)
            if matches!(&member.prop, MemberProp::Computed(computed)
                if member_property_name(&member.prop).is_none()
                    && well_known_symbol_from_expr(&computed.expr).is_none())
            && matches!(self.peek_type_without_lowering(&member.obj),
                Some(HirType::Array(element)) if matches!(*element,
                    HirType::Function(..) | HirType::CallableFunction(..) | HirType::OverloadedFunction(..))));
        if indexed_function_element
            || (matches!(callee_expr.as_ref(), Expr::Ident(identifier)
                if is_quickjs_function_value(identifier.span())
                    && call.args.iter().any(|argument| argument.spread.is_some())))
            || !matches!(callee_expr.as_ref(), Expr::Ident(_) | Expr::Member(_))
        {
            if call.type_args.is_some() {
                return Err("function-value invocation does not accept type arguments".into());
            }
            let quickjs_function_value = self
                .validate_quickjs_function_value_arguments(callee_expr, &call.args)?;
            let callee = self.lower_expr(callee_expr)?;
            let callee_ty = self.infer_expr_type(&callee)?;
            if matches!(callee_ty, HirType::JsValue | HirType::Dynamic) {
                return self.lower_dynamic_value_call_on(callee, &call.args);
            }
            let (public_overloads, physical_ty) = match callee_ty {
                HirType::OverloadedFunction(public, physical) => (Some(public), *physical),
                other => (None, other),
            };
            let (params, optional, rest, physical_return) = match physical_ty {
                HirType::Function(params, ret) => {
                    (params, HirOptionalMask::default(), None, *ret)
                }
                HirType::CallableFunction(params, optional, rest, ret) => {
                    (params, optional, rest.map(|element| *element), *ret)
                }
                _ => return Err("call target is not a function value".into()),
            };
            let selected_public_signature = public_overloads
                .as_deref()
                .map(|overloads| self.select_public_overload_for_source(
                    overloads, &call.args, None, "function value",
                ))
                .transpose()?
                .flatten();
            let selected_return = selected_public_signature
                .as_ref()
                .and_then(|public| self.public_overload_return_type(public));
            let public_rest = selected_public_signature.as_ref().and_then(|public| match public {
                HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                    HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
                    _ => None,
                },
                HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
                _ => None,
            });
            let public_params = selected_public_signature.as_ref().map(|public| {
                (0..params.len())
                    .filter_map(|index| self.public_overload_argument_type(public, index))
                    .collect::<Vec<_>>()
            });
            let (mut arguments, bindings) = if let Some(public_params) = &public_params {
                self.lower_native_spread_values_with_overload_expected(
                    &call.args,
                    "function-value invocation",
                    public_params,
                    public_rest,
                )?
            } else {
                self.lower_native_spread_values(&call.args, "function-value invocation")?
            };
            if let Some(public) = selected_public_signature.as_ref() {
                let public_rest = match public {
                    HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                        HirType::CallableFunction(params, _, Some(_), _) => Some(params.len()),
                        _ => None,
                    },
                    HirType::CallableFunction(params, _, Some(_), _) => Some(params.len()),
                    _ => None,
                };
                if let Some(public_fixed_count) = public_rest.filter(|count| *count < params.len()) {
                    if arguments.len() > public_fixed_count {
                        let public_rest_values = arguments.split_off(public_fixed_count);
                        for parameter in params.iter().skip(public_fixed_count) {
                            arguments.push(omitted_parameter_value(parameter)?);
                        }
                        arguments.extend(public_rest_values);
                    }
                }
            }
            // JS evaluates and ignores surplus arguments.
            // ponytail: surplus args are evaluated before the kept ones, so a
            // side-effecting surplus arg reorders against side-effecting kept args.
            let dropped_extras = if rest.is_none() && arguments.len() > params.len() {
                arguments.split_off(params.len())
            } else {
                Vec::new()
            };
            if arguments.len() < params.len()
                && (arguments.len()..params.len())
                    .any(|index| !is_optional_parameter(&optional, index))
            {
                return Err(format!(
                    "function value expects at least {} argument(s), got {}",
                    params.len(),
                    arguments.len()
                ));
            }
            let rest_values = rest.as_ref().map(|element| {
                let values = arguments.split_off(arguments.len().min(params.len()));
                (element, values)
            });
            for parameter in params.iter().skip(arguments.len()) {
                arguments.push(omitted_parameter_value(parameter)?);
            }
            let mut arguments = arguments
                .into_iter()
                .zip(&params)
                .enumerate()
                .map(|(index, (argument, expected))| {
                    let public_expected = selected_public_signature.as_ref().and_then(|public| {
                        self.public_overload_argument_type(public, index)
                    });
                    let argument = self.adapt_selected_overload_argument(
                        expected,
                        public_expected.as_ref(),
                        argument,
                    )?;
                    if quickjs_function_value {
                        self.coerce_primitive_array_argument_aliasing(argument, expected)
                    } else {
                        self.coerce_to_declared(expected, argument)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some((element, values)) = rest_values {
                let values = values
                    .into_iter()
                    .map(|value| self.coerce_to_declared(element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                arguments.push(native_rest_array(values, element));
            }
            let result = HirExpr::Call(Box::new(callee), arguments);
            let result = if let Some(selected) = selected_return {
                self.project_overload_call_result(result, &physical_return, &selected)?
            } else {
                result
            };
            let result = dropped_extras
                .into_iter()
                .rev()
                .fold(result, |inner, extra| HirExpr::EvalThen(Box::new(extra), Box::new(inner)));
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if let Some(bound) = self.lower_native_class_bind(call)? {
            return Ok(bound);
        }
        if let Some(bound) = self.lower_native_static_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_native_class_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_native_static_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_this_parameter_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_object_prototype_to_string_call(call)? {
            return Ok(invoked);
        }
        if let Some(generic) = self.lower_generic_arrow_operation(
            call,
            expected_source_return_hint.as_ref(),
            expected_return_hint.as_ref(),
        )? {
            return Ok(generic);
        }
        if let Some(bound) = self.lower_function_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_function_call_or_apply(call)? {
            return Ok(invoked);
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let Some(property) = member_property_name(&member.prop) {
                if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                    let class_name = self
                        .class_context
                        .as_deref()
                        .expect("static class lowering retains its class context");
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                if let Expr::Ident(class) = member.obj.as_ref() {
                    let class_name = class.sym.as_ref();
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.is_unshadowed_class_identifier(class_name)
                        && self.signatures.contains_key(&symbol)
                    {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                let receiver_type = self.infer_member_receiver_type(&member.obj);
                // `stack.filter(..).map(..)`: the receiver is another species result; bind it so
                // the call sees its class (and wraps its own result in turn).
                if receiver_type.is_none() && self.is_array_subclass_chain(&member.obj) {
                    let chained = self.lower_expr(&member.obj)?;
                    let chained_type = self.infer_expr_type(&chained)?;
                    if native_base_type(&chained_type).is_some() {
                        let name = format!("__thaw_chain_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), chained_type.clone());
                        let mut rewritten = call.clone();
                        if let Callee::Expr(callee) = &mut rewritten.callee {
                            if let Expr::Member(rewritten_member) = callee.as_mut() {
                                *rewritten_member.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                    name.as_str().into(),
                                    call.span,
                                ));
                            }
                        }
                        let result = self.lower_call(&rewritten)?;
                        return self.wrap_call_argument_bindings(result, &[(name, chained_type, chained)]);
                    }
                }
                if let Some(class_receiver_type) = receiver_type.clone().filter(|ty| {
                    class_name_from_type(ty).is_some()
                        && !Self::class_receiver_has_own_field(ty, &property)
                }) {
                    let class_name = class_name_from_type(&class_receiver_type)
                        .expect("the receiver was classified as a native class");
                    let symbol = class_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native class method `{class_name}.{property}` is not generic"
                            ));
                        }
                        let overrides = self.virtual_method_overrides(class_name, &property);
                        if !overrides.is_empty() && !self.class_static_context {
                            return self.lower_virtual_class_method_call(
                                call,
                                member,
                                &class_receiver_type,
                                &property,
                                &overrides,
                                false,
                            );
                        }
                        let mut args = Vec::with_capacity(call.args.len() + 1);
                        args.push(swc_ecma_ast::ExprOrSpread {
                            spread: None,
                            expr: member.obj.clone(),
                        });
                        args.extend(call.args.iter().cloned());
                        return self.lower_call_with_class_guard(
                            &CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args,
                            type_args: None,
                            },
                            Some(class_name),
                        );
                    }
                    // A class extending `Map`/`Set` forwards every method it does
                    // not define itself to the wrapped native collection.
                    if native_base_type(&class_receiver_type).is_some() {
                        // A subclass that defines the method itself overrides it for
                        // instances reaching here through the base type.
                        let overrides = self
                            .native_subclasses(class_name)
                            .into_iter()
                            .filter(|(name, _)| {
                                self.signatures
                                    .get(&class_method_symbol(name, &property))
                                    .is_some_and(|own| own.accessor_owner.as_deref() == Some(name.as_str()))
                            })
                            .collect::<Vec<_>>();
                        if !overrides.is_empty() && !self.class_static_context {
                            return self.lower_virtual_class_method_call(
                                call,
                                member,
                                &class_receiver_type,
                                &property,
                                &overrides,
                                true,
                            );
                        }
                        let mut forwarded = call.clone();
                        let receiver = (*member.obj).clone();
                        if let Callee::Expr(callee) = &mut forwarded.callee {
                            if let Expr::Member(forwarded_member) = callee.as_mut() {
                                *forwarded_member.obj = native_base_member(receiver.clone());
                            }
                        }
                        return self.lower_native_base_forward(&forwarded, &property, &receiver);
                    }
                    // Date/Object builtins (`getTime`, `hasOwnProperty`, ...) are not
                    // class methods; they reach the native builtin dispatch below.
                    if !Self::is_native_instance_builtin(&property) {
                        return Err(format!(
                            "class `{class_name}` has no native method `{property}`"
                        ));
                    }
                }
                // A receiver typed `JsValue` (an opaque handle -- a
                // Fallback function's return value that couldn't be
                // classified as anything JSON-representable, e.g. zod's
                // `z.object(...)` returning a live `ZodObject` schema
                // instance) has no known class/method table the way the
                // branch above needs, but calling a method on it by name
                // is still meaningful: the value really is a live JS
                // object on the other side of the QuickJS boundary, it's
                // just one thaw has no compiled knowledge of the shape
                // of. Real example: that same schema's own
                // `.safeParse(data)`. Routes through `callDynamicMethod`
                // (an existing low-level intrinsic, previously only a
                // manual escape hatch a user could call directly)
                // instead of erroring the way an unrecognized *class*
                // method above does, since there's no fixed method list
                // to have missed from.
                //
                // A Json `.includes()` receiver is routed separately below:
                // it may carry a borrowed handle or be an ordinary decoded
                // value, and the helper handles both without handing a Json
                // pointer to an API that requires a live handle.
                if property == "includes"
                    && receiver_type == Some(HirType::Json)
                    && !call.args.iter().any(|argument| argument.spread.is_some())
                {
                    return self.lower_json_includes_method_call(
                        member,
                        &call.args,
                        expected_return_hint.as_ref(),
                    );
                }
                if matches!(receiver_type, Some(HirType::JsValue)) {
                    return self.lower_dynamic_value_method_call(
                        &member.obj,
                        &property,
                        &call.args,
                        expected_return_hint.as_ref(),
                    );
                }
            }
        }

        if matches!(callee_expr.as_ref(), Expr::Ident(identifier) if identifier.sym == *"__thaw_object_rest")
        {
            if call.args.is_empty() || call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err("object-rest lowering expects a source and static field names".into());
            }
            let source = self.lower_expr(&call.args[0].expr)?;
            let source_type = self.infer_expr_type(&source)?;
            if let HirType::Dictionary(element) = &source_type {
                let omitted = call.args[1..]
                    .iter()
                    .map(|argument| {
                        let key = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_string(key)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let copy = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_object_assign".into())),
                    vec![
                        HirExpr::JsonObjectLit(Vec::new(), element.as_ref().clone()),
                        source,
                    ],
                );
                let parameter = format!("__thaw_object_rest_copy_{}", self.next_binding);
                self.next_binding += 1;
                let mut parameters = vec![HirParam {
                    name: parameter.clone(),
                    ty: source_type.clone(),
                }];
                let mut arguments = vec![copy];
                let mut body = Vec::with_capacity(omitted.len() + 1);
                for key in omitted {
                    let key_parameter = format!("__thaw_object_rest_key_{}", self.next_binding);
                    self.next_binding += 1;
                    parameters.push(HirParam {
                        name: key_parameter.clone(),
                        ty: HirType::Str,
                    });
                    arguments.push(key);
                    body.push(HirStmt::Expr(HirExpr::JsonDelete(
                        Box::new(HirExpr::Var(parameter.clone())),
                        Box::new(HirExpr::Var(key_parameter)),
                    )));
                }
                body.push(HirStmt::Return(Some(HirExpr::Var(parameter.clone()))));
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        Vec::new(),
                        parameters,
                        source_type,
                        Box::new(HirExpr::Block(body)),
                    )),
                    arguments,
                ));
            }
            let HirType::Object(fields) = &source_type else {
                return Err(format!(
                    "object rest requires a fixed-shape object, got {source_type:?}"
                ));
            };
            let omitted = call.args[1..]
                .iter()
                .map(|argument| match argument.expr.as_ref() {
                    Expr::Lit(Lit::Str(key)) => Ok(key.value.to_string_lossy().into_owned()),
                    _ => Err("object-rest field names must be string literals".to_string()),
                })
                .collect::<Result<HashSet<_>, _>>()?;
            let omitted = omitted.into_iter().collect::<Vec<_>>();
            return self
                .lower_fixed_object_rest_copy(source, fields, &omitted)
                .map(|(value, _)| value);
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(property) = &member.prop {
                if let Expr::Ident(object) = member.obj.as_ref() {
                    if object.sym == *"ArrayBuffer" && property.sym == *"isView" {
                        if let Some(native) = self.lower_native_array_buffer_is_view(call)? {
                            return Ok(native);
                        }
                    }
                    if Self::is_static_builtin_call(object.sym.as_ref(), property.sym.as_ref()) {
                        return self.lower_static_builtin_call(object, property, call);
                    }
                }
                // A live Json host receiver retains its original method lookup
                // and this binding. Native Date values keep the timestamp path.
                // Ordinary native Json method storage is a separate boundary.
                if matches!(
                    property.sym.as_ref(),
                    "getTime"
                        | "setTime"
                        | "setYear"
                        | "toISOString"
                        | "getFullYear"
                        | "getMonth"
                        | "getDate"
                        | "getDay"
                        | "getHours"
                        | "getMinutes"
                        | "getSeconds"
                        | "getMilliseconds"
                        | "getTimezoneOffset"
                        | "getUTCFullYear"
                        | "getUTCMonth"
                        | "getUTCDate"
                        | "getUTCDay"
                        | "getUTCHours"
                        | "getUTCMinutes"
                        | "getUTCSeconds"
                        | "getUTCMilliseconds"
                        | "setFullYear"
                        | "setMonth"
                        | "setDate"
                        | "setHours"
                        | "setMinutes"
                        | "setSeconds"
                        | "setMilliseconds"
                        | "setUTCFullYear"
                        | "setUTCMonth"
                        | "setUTCDate"
                        | "setUTCHours"
                        | "setUTCMinutes"
                        | "setUTCSeconds"
                        | "setUTCMilliseconds"
                ) && matches!(
                    self.peek_type_without_lowering(&member.obj),
                    Some(HirType::Json)
                ) {
                    let json_receiver = self.lower_expr(&member.obj)?;
                    let json_name = format!("__thaw_date_json_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(json_name.clone(), HirType::Json);
                    // Keep each spread source inside its selected branch.
                    let (arguments, spread_bindings) = self.lower_native_spread_values(
                        &call.args,
                        &format!("Date.{}", property.sym),
                    )?;
                    // The live branch builds its argument expression separately:
                    // the dynamic call's selected-property lookup runs before
                    // this expression evaluates any source or tuple spread.
                    let live_elements = arguments
                        .iter()
                        .cloned()
                        .map(|value| self.coerce_to_declared(&HirType::Json, value))
                        .collect::<Result<Vec<_>, _>>()?;
                    let live_array =
                        self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(live_elements))?;
                    let live_array =
                        self.wrap_call_argument_bindings(live_array, &spread_bindings)?;
                    let mut bindings = spread_bindings;
                    let mut synthetic_call = call.clone();
                    synthetic_call.args.clear();
                    for argument in arguments {
                        let ty = self.infer_expr_type(&argument)?;
                        let name = format!("__thaw_date_json_arg_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        bindings.push((name.clone(), ty, argument));
                        synthetic_call.args.push(swc_ecma_ast::ExprOrSpread {
                            spread: None,
                            expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                                name.into(),
                                member.span,
                            ))),
                        });
                    }
                    let timestamp_name =
                        format!("__thaw_json_date_timestamp_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(timestamp_name.clone(), HirType::F64);
                    bindings.push((
                        timestamp_name.clone(),
                        HirType::F64,
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_date_timestamp".to_string())),
                            vec![HirExpr::Var(json_name.clone())],
                        ),
                    ));
                    let promoted_name = format!("__thaw_promoted_date_{}", self.next_binding);
                    self.next_binding += 1;
                    let date_type = date_object_type();
                    self.scope.insert(promoted_name.clone(), date_type.clone());
                    let local_date = format!("__thaw_promoted_date_alloc_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(local_date.clone(), date_type.clone());
                    let promoted_object = HirExpr::Block(vec![
                        HirStmt::Let(
                            local_date.clone(),
                            date_type.clone(),
                            HirExpr::ClassAlloc(date_type.clone()),
                        ),
                        HirStmt::Expr(HirExpr::PropAssign(
                            Box::new(HirExpr::Var(local_date.clone())),
                            date_type.clone(),
                            "timestamp".to_string(),
                            Box::new(HirExpr::Var(timestamp_name)),
                        )),
                        HirStmt::Return(Some(HirExpr::Var(local_date))),
                    ]);
                    bindings.push((promoted_name.clone(), date_type, promoted_object));
                    let promoted_ident =
                        swc_ecma_ast::Ident::new_no_ctxt(promoted_name.into(), member.span);
                    let synthetic_member = MemberExpr {
                        obj: Box::new(Expr::Ident(promoted_ident)),
                        ..member.clone()
                    };
                    let method_call = self.lower_native_instance_builtin(
                        &synthetic_member,
                        property,
                        &synthetic_call,
                    )?;
                    let result = if matches!(
                        property.sym.as_ref(),
                        "setTime"
                            | "setYear"
                            | "setFullYear"
                            | "setMonth"
                            | "setDate"
                            | "setHours"
                            | "setMinutes"
                            | "setSeconds"
                            | "setMilliseconds"
                            | "setUTCFullYear"
                            | "setUTCMonth"
                            | "setUTCDate"
                            | "setUTCHours"
                            | "setUTCMinutes"
                            | "setUTCSeconds"
                            | "setUTCMilliseconds"
                    ) {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_date_set_timestamp".into())),
                            vec![HirExpr::Var(json_name.clone()), method_call],
                        )
                    } else {
                        method_call
                    };
                    let native = self.wrap_call_argument_bindings(result, &bindings)?;
                    let wants_handle = matches!(
                        expected_return_hint.as_ref(),
                        Some(HirType::JsValue | HirType::Dynamic)
                    );
                    let native_json = self.coerce_to_declared(&HirType::Json, native)?;
                    let native = if wants_handle {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("retainDynamicJson".into())),
                            vec![native_json],
                        )
                    } else {
                        native_json
                    };
                    let handle_name =
                        format!("__thaw_date_live_method_handle_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(handle_name.clone(), HirType::I64);
                    let live_intrinsic = match expected_return_hint.as_ref() {
                        Some(HirType::Dynamic) => "__thaw_call_selected_dynamic_method_raw",
                        Some(HirType::JsValue) => "__thaw_call_selected_dynamic_method_handle",
                        _ => "__thaw_call_selected_dynamic_method",
                    };
                    let live = HirExpr::Call(
                        Box::new(HirExpr::Var(live_intrinsic.into())),
                        vec![
                            HirExpr::TypedClosure(
                                HirType::JsValue,
                                Box::new(HirExpr::Var(handle_name.clone())),
                            ),
                            HirExpr::Lit(HirLit::Str(property.sym.to_string())),
                            live_array.clone(),
                        ],
                    );
                    let result_type = if wants_handle {
                        HirType::JsValue
                    } else {
                        HirType::Json
                    };
                    let branch = HirExpr::Conditional(
                        Box::new(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Var(handle_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::I64(0))),
                        )),
                        Box::new(native),
                        Box::new(live),
                        result_type,
                    );
                    return self.wrap_call_argument_bindings(
                        branch,
                        &[
                        (json_name.clone(), HirType::Json, json_receiver),
                            (
                                handle_name,
                                HirType::I64,
                                HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_json_borrowed_handle_id".into())),
                            vec![HirExpr::Var(json_name)],
                                ),
                            ),
                        ],
                    );
                }
                // A confirmed `JsValue` receiver (real trigger: a Date-
                // valued Fallback return, see `[[project_npm_interop_
                // gaps_19]]`) is never a native thaw value under any
                // circumstance, so a name this codebase also happens to
                // use for a native builtin (`toISOString`, `getTime`, ...)
                // must fall through to the generic dynamic method-call
                // path below instead of `lower_native_instance_builtin`,
                // which unconditionally assumes a native receiver shape
                // and hard-errors otherwise. `peek_type_without_lowering`
                // returning `None` (receiver type not statically
                // knowable without lowering, e.g. itself a call
                // expression) leaves this exactly as permissive as
                // before -- only a *positively confirmed* dynamic
                // receiver changes the outcome.
                //
                // Deliberately `JsValue` only, not `Json`: a `Json`
                // receiver (real trigger: glob's async `glob(...)`
                // resolving to a plain JSON array) still has a real,
                // useful native-builtin error path below (e.g. `.sort`'s
                // own "requires an array receiver, got Json") -- skipping
                // straight to the dynamic path instead left it with no
                // path at all (`lower_dynamic_value_method_call` also
                // requires `HirType::JsValue`, further down), regressing
                // a working error into "call to undeclared function".
                let receiver_type = self.peek_type_without_lowering(&member.obj);
                let private_primitive_receiver = matches!(
                    receiver_type.as_ref(),
                    Some(HirType::NonArrowReceiver(inner))
                        if matches!(inner.as_ref(), HirType::Str | HirType::I64 | HirType::BigInt)
                );
                let receiver_is_dynamic = receiver_type.as_ref() == Some(&HirType::JsValue)
                    || private_primitive_receiver;
                // Temporal instance methods. A receiver that's itself a
                // Temporal-returning call (a chained `instant.add(...).toString()`)
                // can't be typed by `peek_type_without_lowering`, so those
                // method names are attempted directly; `lower_temporal_method`
                // returns `None` (having only built a pure expression) when
                // the receiver turns out not to be Temporal.
                let temporal_method = matches!(
                    property.sym.as_ref(),
                    "equals"
                        | "add"
                        | "subtract"
                        | "since"
                        | "until"
                        | "total"
                        | "balance"
                        | "abs"
                        | "negated"
                        | "round"
                        | "toPlainDate"
                        | "toPlainDateTime"
                        | "toPlainTime"
                        | "toInstant"
                        | "startOfDay"
                        | "getTimeZoneTransition"
                        | "with"
                        | "withTimeZone"
                        | "withCalendar"
                        | "toZonedDateTimeISO"
                        | "toZonedDateTime"
                );
                let generic_method = matches!(property.sym.as_ref(), "toString" | "toJSON");
                let receiver_maybe_temporal = self
                    .peek_type_without_lowering(&member.obj)
                    .map(|ty| Self::temporal_kind(&ty).is_some())
                    .unwrap_or(matches!(member.obj.as_ref(), Expr::Call(_) | Expr::New(_)));
                if temporal_method || (generic_method && receiver_maybe_temporal) {
                    if let Some(result) = self.lower_temporal_method(member, property, call)? {
                        return Ok(result);
                    }
                }
                let own_callable = self
                    .peek_type_without_lowering(&member.obj)
                    .is_some_and(|ty| matches!(ty,
                        HirType::Object(fields) if fields.iter().any(|(name, field)|
                            name == property.sym.as_ref()
                                && matches!(field, HirType::Function(..) | HirType::CallableFunction(..))
                        )
                    ));
                let defer_native = self.peek_type_without_lowering(&member.obj).is_none()
                    && matches!(member.obj.as_ref(), Expr::Call(_) | Expr::New(_));
                if (Self::is_native_instance_builtin(property.sym.as_ref())
                    || (Self::is_native_view_only_method(property.sym.as_ref())
                        && self.receiver_is_bytes(&member.obj)))
                    && !receiver_is_dynamic
                    && !own_callable
                    && !defer_native
                    && self.console_method_value(member).is_none()
                {
                    return self.lower_native_instance_builtin(member, property, call);
                }
                // `.get()`/`.has()` on a `Json`-typed (`any`) receiver
                // reach the same dispatch too, for the read-only subset
                // that's actually implemented for the `__thaw_map_
                // entries__`/`__thaw_set_values__` sentinel wrapper --
                // `lower_native_map_set_method` degrades the rest
                // (`.set`/`.delete`/`.add`/`.clear`/...) to a clear
                // "not supported on any" error instead of leaving them
                // to a far vaguer "call to unknown function" (this was
                // already true for a *statically* mismatched receiver
                // before this addition; a `Json` one is no different).
                if matches!(
                    property.sym.as_ref(),
                    "get"
                        | "set"
                        | "has"
                        | "delete"
                        | "add"
                        | "clear"
                        | "getOrInsert"
                        | "getOrInsertComputed"
                ) && (self.receiver_is_map_or_set(&member.obj)
                    || self.peek_type_without_lowering(&member.obj) == Some(HirType::Json)
                    || (property.sym == *"set" && self.receiver_is_bytes(&member.obj)))
                {
                    return self.lower_native_instance_builtin(member, property, call);
                }
                if matches!(property.sym.as_ref(), "finally" | "then" | "catch") {
                    return self.lower_promise_member_call(member, property, call);
                }
            }
        }

        // An object field with a function type is a callable value. Preserve
        // it as `Call(PropAccess(...), args)` instead of flattening it into a
        // synthetic `object.method` global symbol (the latter is reserved for
        // builtins such as `console.log` and `JSON.parse`).
        if let Expr::Member(member) = callee_expr.as_ref() {
            let property = member_property_name(&member.prop);
            if let Some(property) = property {
                let lowered_object = if let Expr::Ident(object) = member.obj.as_ref() {
                    let object_name = self.resolve_binding(object.sym.as_ref());
                    let object_ty = self
                        .narrowings
                        .get(&object_name)
                        .cloned()
                        .or_else(|| self.nullable_narrowings.get(&object_name).cloned())
                        .or_else(|| self.nullish_narrowings.get(&object_name).cloned())
                        .or_else(|| {
                            self.partial_nullable_narrowings
                                .get(&object_name)
                                .map(|payload| HirType::Nullable(Box::new(payload.clone())))
                        })
                        .or_else(|| self.scope.get(&object_name).cloned());
                    object_ty
                        .map(|object_ty| {
                            self.lower_expr(&member.obj)
                                .map(|object_expr| (object_expr, object_ty, object.sym.to_string()))
                        })
                        .transpose()?
                } else {
                    let object_expr = self.lower_expr(&member.obj)?;
                    let object_ty = self.infer_expr_type(&object_expr)?;
                    Some((object_expr, object_ty, "<expression>".to_string()))
                };
                if let Some((object_expr, object_ty, object_label)) = lowered_object {
                    let requested_property = property.as_str();
                    // An element read that only looks optional (the native types are more
                    // conservative than TypeScript's) before calling a method-bearing object.
                    let (object_expr, object_ty) = match &object_ty {
                        HirType::Optional(payload) if matches!(payload.as_ref(), HirType::Object(fields)
                            if fields.iter().any(|(name, ty)| name == requested_property
                                && matches!(ty, HirType::Function(..) | HirType::CallableFunction(..)))) =>
                        {
                            let strict = self.receiver_requires_narrowing(&member.obj);
                            self.unwrap_required_optional_member(object_expr, object_ty, requested_property, strict)?
                        }
                        _ => (object_expr, object_ty),
                    };
                    if matches!(object_expr, HirExpr::NonArrowThis { .. })
                        || matches!(object_ty, HirType::NonArrowReceiver(_))
                    {
                        let callable_shape = match (&object_expr, &object_ty) {
                            (HirExpr::NonArrowThis { declared, .. }, _) => declared,
                            (_, HirType::NonArrowReceiver(inner)) => inner.as_ref(),
                            (_, ty) => ty,
                        };
                        let plain_callable = matches!(callable_shape, HirType::Object(fields)
                            if fields.iter().any(|(name, ty)| name == requested_property
                                && matches!(ty, HirType::Function(..) | HirType::CallableFunction(..))));
                        if plain_callable {
                            return self.lower_dynamic_value_method_call(
                                &member.obj,
                                requested_property,
                                &call.args,
                                expected_return_hint.as_ref(),
                            );
                        }
                        let primitive_receiver = matches!(callable_shape, HirType::Str | HirType::I64 | HirType::BigInt);
                        if primitive_receiver && Self::is_native_instance_builtin(requested_property) {
                            return self.lower_dynamic_value_method_call(
                                &member.obj,
                                requested_property,
                                &call.args,
                                expected_return_hint.as_ref(),
                            );
                        }
                    }
                    let resolved_property = match (requested_property, call.args.len()) {
                        ("listen", 2) => "__listenWithCallback",
                        ("listen", 3) => "__listenWithHostCallback",
                        ("close", 1) => "__closeWithCallback",
                        ("on", 2)
                            if matches!(
                                call.args.first().map(|arg| arg.expr.as_ref()),
                                Some(Expr::Lit(Lit::Str(event))) if event.value == *"error"
                            ) =>
                        {
                            "__onError"
                        }
                        _ => requested_property,
                    };
                    // A function-valued field widened to `Optional` because it may
                    // be uninitialised: call through its payload, throwing Node's
                    // `TypeError` while it is still absent.
                    let uninit_callable = match &object_ty {
                        HirType::Object(fields) if is_uninit_class_field(fields, resolved_property) => fields
                            .iter()
                            .find(|(name, _)| name == resolved_property)
                            .and_then(|(_, ty)| match ty {
                                HirType::Optional(payload) => Some(payload.as_ref().clone()),
                                _ => None,
                            }),
                        _ => None,
                    };
                    let callable = match &object_ty {
                        HirType::Object(fields) => fields
                            .iter()
                            .find(|(name, _)| name == resolved_property)
                            .and_then(|(_, ty)| match uninit_callable.as_ref().unwrap_or(ty) {
                                HirType::Function(params, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    None,
                                    HirOptionalMask::default(),
                                    None,
                                )),
                                HirType::CallableFunction(params, optional, rest, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    rest.as_deref().cloned(),
                                    optional.clone(),
                                    None,
                                )),
                                HirType::OverloadedFunction(public, physical) => match physical.as_ref() {
                                    HirType::Function(params, ret) => Some((
                                        params.clone(), ret.as_ref().clone(), None,
                                        HirOptionalMask::default(), Some(public.clone()),
                                    )),
                                    HirType::CallableFunction(params, optional, rest, ret) => Some((
                                        params.clone(), ret.as_ref().clone(), rest.as_deref().cloned(),
                                        optional.clone(), Some(public.clone()),
                                    )),
                                    _ => None,
                                },
                                _ => None,
                            }),
                        _ => None,
                    };
                    // A live `JsValue` receiver that only became visible by
                    // lowering it (``new Intl.NumberFormat(...).format(...)``,
                    // a chained dynamic call) has no compiled field table;
                    // dispatch to the dynamic method call the same way a
                    // peekable `JsValue` binding already does above.
                    if callable.is_none() && object_ty == HirType::JsValue {
                        return self.lower_dynamic_value_method_call(
                            &member.obj,
                            &property,
                            &call.args,
                            expected_return_hint.as_ref(),
                        );
                    }
                    if let Some((params, method_ret, rest, optional, public_overloads)) = callable {
                        // Compile-time marker: keep ordinary function-valued
                        // properties distinct from receiver-aware methods.
                        let receiver_parameter = params.first()
                            .and_then(Self::object_method_receiver_fields).is_some();
                        let receiver_name = format!("__thaw_method_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), object_ty.clone());
                        let receiver = HirExpr::Var(receiver_name.clone());
                        let mut callee = HirExpr::PropAccess(
                            Box::new(receiver.clone()),
                            object_ty.clone(),
                            resolved_property.to_string(),
                        );
                        if let Some(payload) = &uninit_callable {
                            let raw_name = format!("__thaw_uninit_callee_{}", self.next_binding);
                            self.next_binding += 1;
                            let optional = HirType::Optional(Box::new(payload.clone()));
                            self.scope.insert(raw_name.clone(), optional.clone());
                            let raw = HirExpr::Var(raw_name.clone());
                            let checked = HirExpr::Conditional(
                                Box::new(HirExpr::OptionalIsNone(Box::new(raw.clone()), payload.clone())),
                                Box::new(HirExpr::ThrowValue(
                                    Box::new(HirExpr::Lit(HirLit::Str(format!(
                                        "\u{1}TypeError\u{1}{object_label}.{property} is not a function"
                                    )))),
                                    Box::new(HirExpr::TypedClosure(
                                        payload.clone(),
                                        Box::new(Self::unreachable_value(payload)?),
                                    )),
                                )),
                                Box::new(HirExpr::OptionalValue(Box::new(raw), payload.clone())),
                                payload.clone(),
                            );
                            callee = self.wrap_call_argument_bindings(checked, &[(raw_name, optional, callee)])?;
                        }
                        let callee_type = self.infer_expr_type(&callee)?;
                        let callee_name = format!("__thaw_method_callable_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(callee_name.clone(), callee_type.clone());
                        let label = format!("method `{object_label}.{property}`");
                        let expected = if receiver_parameter {
                            &params[1..]
                        } else {
                            &params[..]
                        };
                        let selected_public_signature = public_overloads
                            .as_deref()
                            .map(|overloads| {
                                self.select_public_overload_for_source(
                                    overloads,
                                    &call.args,
                                    None,
                                    &label,
                                )
                            })
                            .transpose()?
                            .flatten();
                        let selected_public_return = selected_public_signature
                            .as_ref()
                            .and_then(|public| self.public_overload_return_type(public));
                        let public_rest = selected_public_signature.as_ref().and_then(|public| match public {
                            HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                                HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
                                _ => None,
                            },
                            HirType::CallableFunction(_, _, rest, _) => rest.as_deref(),
                            _ => None,
                        });
                        let public_params = selected_public_signature.as_ref().map(|public| {
                            (0..expected.len())
                                .filter_map(|index| self.public_overload_visible_argument_type(public, index))
                                .collect::<Vec<_>>()
                        });
                        let (mut args, mut bindings) = if let Some(public_params) = &public_params {
                            self.lower_native_spread_values_with_overload_expected(
                                &call.args,
                                &label,
                                public_params,
                                public_rest,
                            )?
                        } else {
                            self.lower_native_spread_values_with_expected(
                                &call.args,
                                &label,
                                expected,
                                rest.as_ref(),
                            )?
                        };
                        if let Some(public) = selected_public_signature.as_ref() {
                            let visible_params = if receiver_parameter { &params[1..] } else { &params[..] };
                            let public_rest_start = match public {
                                HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                                    HirType::CallableFunction(public_params, _, Some(_), _) => Some(public_params.len()),
                                    _ => None,
                                },
                                HirType::CallableFunction(public_params, _, Some(_), _) => Some(public_params.len()),
                                _ => None,
                            };
                            if let Some(public_rest_start) = public_rest_start.filter(|start| *start < visible_params.len()) {
                                if args.len() > public_rest_start {
                                    let rest_values = args.split_off(public_rest_start);
                                    for parameter in visible_params.iter().skip(public_rest_start) {
                                        args.push(omitted_parameter_value(parameter)?);
                                    }
                                    args.extend(rest_values);
                                }
                            }
                        }
                        bindings.insert(0, (receiver_name.clone(), object_ty.clone(), object_expr));
                        bindings.insert(1, (callee_name.clone(), callee_type, callee));
                        let mut copied_receiver = None;
                        let mut copied_receiver_view_type = None;
                        if receiver_parameter {
                            let receiver_fields = Self::object_method_receiver_fields(&params[0]).unwrap();
                            let receiver_view_type = HirType::Object(receiver_fields.to_vec());
                            let (this_arg, view_binding) = self.native_object_method_receiver(
                                receiver.clone(),
                                &object_ty,
                                receiver_fields,
                            )?;
                            if let Some(binding) = view_binding {
                                bindings.insert(2, binding);
                                args.insert(0, HirExpr::NonArrowReceiverPack {
                                    value: Box::new(this_arg), declared: receiver_view_type,
                                });
                            } else {
                                // The method got a copy of the owner (it uses `this` as a
                                // value): bind it so the call can copy it back afterwards.
                                let copy_name =
                                    format!("__thaw_method_receiver_copy_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(copy_name.clone(), receiver_view_type.clone());
                                bindings
                                    .insert(2, (copy_name.clone(), receiver_view_type.clone(), this_arg));
                                args.insert(0, HirExpr::NonArrowReceiverPack {
                                    value: Box::new(HirExpr::Var(copy_name.clone())), declared: receiver_view_type.clone(),
                                });
                                copied_receiver = Some(copy_name);
                                copied_receiver_view_type = Some(receiver_view_type);
                            }
                        }
                        if args.len() < params.len()
                            && (args.len()..params.len())
                                .any(|index| !is_optional_parameter(&optional, index))
                        {
                            return Err(format!(
                                "method `{}.{}` expects at least {} argument(s), got {}",
                                object_label,
                                property,
                                params.len(),
                                args.len()
                            ));
                        }
                        let rest_values = rest.as_ref().map(|element| {
                            let values = args.split_off(args.len().min(params.len()));
                            (element, values)
                        });
                        for parameter in params.iter().skip(args.len()) {
                            args.push(omitted_parameter_value(parameter)?);
                        }
                        let mut args = args
                            .into_iter()
                            .zip(&params)
                            .enumerate()
                            .map(|(index, (value, expected))| {
                                let public_expected = selected_public_signature.as_ref().and_then(|public| {
                                    self.public_overload_visible_argument_type(public, index)
                                });
                                let value = self.adapt_selected_overload_argument(
                                    expected,
                                    public_expected.as_ref(),
                                    value,
                                )?;
                                self.coerce_to_declared(expected, value).map_err(|error| {
                                    format!(
                                        "argument {} of `{}.{}` is invalid: {error}",
                                        index + 1,
                                        object_label,
                                        property
                                    )
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if let Some((element, values)) = rest_values {
                            let values = values
                                .into_iter()
                                .map(|value| self.coerce_to_declared(element, value))
                                .collect::<Result<Vec<_>, _>>()?;
                            args.push(native_rest_array(values, element));
                        }
                        let mut call_expr =
                            HirExpr::Call(Box::new(HirExpr::Var(callee_name)), args);
                        if let Some(selected) = selected_public_return {
                            call_expr = self.project_overload_call_result(
                                call_expr,
                                &method_ret,
                                &selected,
                            )?;
                        }
                        if let Some(copy_name) = copied_receiver {
                            call_expr = self.copy_back_method_receiver(
                                call_expr,
                                &receiver_name,
                                &object_ty,
                                &copy_name,
                                copied_receiver_view_type.as_ref().expect("copied method has a receiver view"),
                                &method_ret,
                            )?;
                        }
                        return self.wrap_call_argument_bindings(call_expr, &bindings);
                    }
                }
            }
        }

        // Call-expression receivers need their returned object type before a
        // same-named builtin can claim the method.
        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(property) = &member.prop {
                if self.peek_type_without_lowering(&member.obj).is_none()
                    && matches!(member.obj.as_ref(), Expr::Call(_) | Expr::New(_))
                    && Self::is_native_instance_builtin(property.sym.as_ref())
                {
                    return self.lower_native_instance_builtin(member, property, call);
                }
            }
        }

        // `obj[Symbol.x](...)` -- dispatch the well-known symbols thaw
        // models natively.
        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Computed(computed) = &member.prop {
                if let Some(symbol) = self.builtin_symbol_member_name(&computed.expr) {
                    return self.lower_well_known_symbol_call(member, symbol, call);
                }
            }
        }

        // A method call on a `Json`/`any` variable that is not a native
        // builtin: the value may be a live native-owner wrapper (a host
        // lease), so route the call to the live object like a `JsValue`
        // receiver. A plain decoded Json value fails at run time instead.
        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(prop) = &member.prop {
                if self.json_receiver_candidate(&member.obj)
                    && !Self::is_native_instance_builtin(prop.sym.as_ref())
                {
                    return self.lower_dynamic_value_method_call(
                        &member.obj,
                        prop.sym.as_ref(),
                        &call.args,
                        expected_return_hint.as_ref(),
                    );
                }
            }
        }

        let source_callee_name = match callee_expr.as_ref() {
            Expr::Ident(ident) => canonical_target
                .map(str::to_owned)
                .unwrap_or_else(|| self.resolve_binding(ident.sym.as_ref())),
            // Console methods have no dedicated HIR node; they are encoded as
            // synthetic names such as "console.log" and codegen special-cases them.
            Expr::Member(member) => {
                let Expr::Ident(obj) = member.obj.as_ref() else {
                    return Err(format!(
                        "unsupported member call target at source bytes {}..{}",
                        member.span.lo.0, member.span.hi.0
                    ));
                };
                let MemberProp::Ident(prop) = &member.prop else {
                    return Err("unsupported member call property".into());
                };
                format!("{}.{}", obj.sym, prop.sym)
            }
            _ => return Err(
                "unsupported call target (only plain identifiers and supported console methods are supported)"
                    .into(),
            ),
        };
        let known_template = canonical_target.is_none().then(|| {
            self.generic_callable_template(&source_callee_name)
        }).flatten();
        let callee_name = match &known_template {
            Some(GenericCallableTemplate::Named { symbol }) => symbol.clone(),
            _ => source_callee_name,
        };
        if callee_name == "__thaw_mark_typed_wrapper_json_argument" {
            return Err("typed-wrapper JSON ownership marker is compiler-private".into());
        }

        if matches!(
            callee_name.as_str(),
            "setTimeout"
                | "clearTimeout"
                | "setInterval"
                | "clearInterval"
                | "setImmediate"
                | "clearImmediate"
        ) && !self.scope.contains_key(&callee_name)
            && !self.signatures.contains_key(&callee_name)
        {
            let binding = format!("__thaw_global_timer_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(binding.clone(), HirType::JsValue);
            let timer_handle = matches!(
                callee_name.as_str(),
                "setTimeout" | "setInterval" | "setImmediate"
            )
            .then_some(HirType::JsValue);
            let native_views_from = match callee_name.as_str() {
                "setTimeout" | "setInterval" => Some(2),
                "setImmediate" => Some(1),
                _ => None,
            };
            let result = self.lower_dynamic_value_call(
                &binding,
                &call.args,
                expected_return_hint.as_ref().or(timer_handle.as_ref()),
                native_views_from,
            )?;
            return self.wrap_call_argument_bindings(
                result,
                &[(
                    binding,
                    HirType::JsValue,
                    HirExpr::Call(
                        Box::new(HirExpr::Var("getDynamicValue".into())),
                        vec![HirExpr::Lit(HirLit::Str(callee_name))],
                    ),
                )],
            );
        }

        // Native callbacks crossing into QuickJS need a live projection for
        // accessor-bearing results. Keep the original native function ABI
        // behind the wrapper; the graph callback marshaller sees its owned
        // Json (or Promise<Json>) result.
        if matches!(
            callee_name.as_str(),
            "registerNativeCallback" | "registerNativeCallbackGraph"
        ) && !self.scope.contains_key(&callee_name)
            && !self.signatures.contains_key(&callee_name)
        {
            if call.type_args.is_some() || call.args.len() != 1 || call.args[0].spread.is_some() {
                return Err("registerNativeCallback expects one non-spread callback".into());
            }
            let callback = self.lower_expr(&call.args[0].expr)?;
            let callback_type = self.infer_expr_type(&callback)?;
            let original_callback_type = callback_type.clone();
            let requested_graph = callee_name == "registerNativeCallbackGraph";
            let (callback, callback_type) =
                self.adapt_callable_result_for_live_projection(callback, callback_type)?;
            let graph_callback = requested_graph || callback_type != original_callback_type;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var(if graph_callback {
                    "registerNativeCallbackGraph".into()
                } else {
                    "registerNativeCallback".into()
                })),
                vec![HirExpr::TypedClosure(callback_type, Box::new(callback))],
            ));
        }

        // `eval(code)` -- an AOT compiler can't evaluate in the compiled
        // scope, so this is lowered to the QuickJS realm's own global
        // `eval` (`callDynamic("eval", [code])`), i.e. the semantics of
        // *indirect* eval: it sees globals, not local bindings.
        if callee_name == "eval"
            && !self.scope.contains_key(&callee_name)
            && !self.signatures.contains_key(&callee_name)
        {
            let values = self.dynamic_call_args_json(&call.args, None)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("callDynamic".into())),
                vec![HirExpr::Lit(HirLit::Str("eval".into())), values],
            ));
        }

        if let Some(bind) = self.deferred_generic_binds.get(&callee_name).cloned() {
            match bind.target {
                GenericCallableTemplate::Arrow { binding, expression } => {
                    let leading_actuals = bind.leading_captures.iter()
                        .map(|capture| HirExpr::Var(capture.name.clone()))
                        .collect();
                    let operation = self.generic_non_arrow_names.contains(&binding).then(|| {
                        GenericArrowThisOperation {
                            receiver: (HirExpr::Var(bind.this_capture.name.clone()), bind.this_capture.ty.clone()),
                            bind: false,
                            ignored: Vec::new(),
                            prelowered: None,
                        }
                    });
                    return self.lower_generic_arrow_call_with_leading_actuals(
                        &binding, &expression, call, operation, leading_actuals,
                    );
                }
                GenericCallableTemplate::Named { symbol } => {
                    let mut forwarded = call.clone();
                    let mut args = bind.leading_captures.iter().map(|capture| swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                            capture.name.clone().into(), capture.source_span.unwrap_or(call.span),
                        ))),
                    }).collect::<Vec<_>>();
                    args.extend(call.args.clone());
                    forwarded.args = args;
                    forwarded.callee = Callee::Expr(Box::new(Expr::Ident(
                        swc_ecma_ast::Ident::new_no_ctxt(symbol.clone().into(), call.span),
                    )));
                    return self.lower_generic_named_call(
                        &forwarded, &symbol, expected_source_return_hint.as_ref(), expected_return_hint.as_ref(),
                    );
                }
            }
        }

        if self.scope.get(&callee_name) == Some(&HirType::JsValue) {
            return self.lower_dynamic_value_call(
                &callee_name,
                &call.args,
                expected_return_hint.as_ref(),
                None,
            );
        }
        // An `any`- (or explicit `unknown`-) annotated callable is stored as
        // `Json`, but the value itself is a `registerNativeCallback` handle
        // placeholder (`coerce_to_declared`'s own `Function`-into-`Json`
        // branch wraps it that way), so it is dynamically callable like a
        // `JsValue` once the handle is recovered. Real trigger: calling an
        // `any`-typed callback parameter -- cors's `CustomOrigin` is
        // `(origin, callback: any) => void`, and `callback(null, origin)`
        // used to fail with "call to unknown function" (`function
        // callIt(cb: any) { cb(41); }` reproduces it with no package at
        // all). Only a plain identifier bound in scope is handled here; a
        // `Json` value that is genuinely data still fails, just at runtime
        // instead of compile time, which is the honest behavior for `any`.
        if self.scope.get(&callee_name) == Some(&HirType::Json) {
            let callee =
                self.coerce_to_declared(&HirType::JsValue, HirExpr::Var(callee_name.clone()))?;
            return self.lower_dynamic_value_call_on(callee, &call.args);
        }

        match known_template.or_else(|| canonical_target.is_none().then(|| self.generic_callable_template(&callee_name)).flatten()) {
            Some(GenericCallableTemplate::Arrow { binding, expression }) => {
                return self.lower_generic_arrow_call(&binding, &expression, call);
            }
            Some(GenericCallableTemplate::Named { .. }) | None => {}
        }

        if matches!(callee_name.as_str(), "isNaN" | "isFinite") {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_number(value.clone())?;
            let result = HirExpr::Call(
                Box::new(HirExpr::Var(
                    if callee_name == "isNaN" {
                        "__thaw_number_is_nan"
                    } else {
                        "__thaw_number_is_finite"
                    }
                    .to_string(),
                )),
                vec![value],
            );
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if callee_name == "structuredClone" {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "structuredClone")?;
            let [value] = arguments.as_slice() else {
                return Err("`structuredClone` expects exactly one argument".into());
            };
            let value = value.clone();
            let value_type = self.infer_expr_type(&value)?;
            let result = self.structured_clone_expr(value, value_type)?;
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if matches!(
            callee_name.as_str(),
            "atob" | "btoa" | "escape" | "unescape"
        ) {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_string(value.clone())?;
            let intrinsic = match callee_name.as_str() {
                "atob" => "__thaw_atob",
                "btoa" => "__thaw_btoa",
                "escape" => "__thaw_escape",
                _ => "__thaw_unescape",
            };
            let result = HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), vec![value]);
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if matches!(
            callee_name.as_str(),
            "encodeURIComponent" | "encodeURI" | "decodeURIComponent" | "decodeURI"
        ) {
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, &callee_name)?;
            let [value] = arguments.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            let value = self.coerce_primitive_to_string(value.clone())?;
            let intrinsic = match callee_name.as_str() {
                "encodeURIComponent" => "__thaw_encode_uri_component",
                "encodeURI" => "__thaw_encode_uri",
                "decodeURIComponent" => "__thaw_decode_uri_component",
                _ => "__thaw_decode_uri",
            };
            let raw_name = format!("__thaw_decode_uri_raw_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(raw_name.clone(), HirType::Str);
            let body = HirExpr::Block(vec![
                HirStmt::Let(
                    raw_name.clone(),
                    HirType::Str,
                    HirExpr::Call(Box::new(HirExpr::Var(intrinsic.to_string())), vec![value]),
                ),
                HirStmt::If(
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_is_null".to_string())),
                        vec![HirExpr::Var(raw_name.clone())],
                    ),
                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                        "\u{1}URIError\u{1}URI malformed".into(),
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
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        if callee_name == "queueMicrotask" {
            let [callback] = call.args.as_slice() else {
                return Err("`queueMicrotask` expects exactly one callback".into());
            };
            if callback.spread.is_some() {
                return Err("`queueMicrotask` does not support a spread callback".into());
            }
            let callback = self.lower_promise_callback(&callback.expr, &[], None)?;
            let HirType::Function(_, callback_output) = self.infer_expr_type(&callback)? else {
                unreachable!()
            };
            let (output, flatten) = match callback_output.as_ref() {
                HirType::Promise(inner) => (inner.as_ref().clone(), true),
                output => (output.clone(), false),
            };
            let resolve_type = HirType::Function(Vec::new(), Box::new(HirType::Void));
            let source = HirExpr::PromiseNew(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: "__thaw_microtask_resolve".into(),
                        ty: resolve_type,
                    }],
                    HirType::Void,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_microtask_resolve".into())),
                        Vec::new(),
                    )),
                )),
                HirType::Void,
                false,
                false,
            );
            let pending = HirExpr::PromiseThen(
                Box::new(source),
                Box::new(callback),
                HirType::Void,
                output,
                false,
                flatten,
            );
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_detach_promise".into())),
                vec![pending],
            ));
        }

        if callee_name == "Promise.withResolvers" {
            return self.lower_promise_with_resolvers(call);
        }
        if callee_name == "Promise.try" {
            return self.lower_promise_try(call);
        }

        if matches!(
            callee_name.as_str(),
            "Promise.all"
                | "Promise.allSettled"
                | "Promise.race"
                | "Promise.any"
                | "Promise.allKeyed"
                | "Promise.allSettledKeyed"
        ) {
            return self.lower_promise_static_call(&callee_name, call);
        }

        if callee_name == "Promise.resolve" || callee_name == "Promise.reject" {
            // Desugars to the executor form (`new Promise((resolve, reject)
            // => resolve(value))` / `=> reject(message)`) instead of adding
            // a dedicated HIR node, reusing PromiseNew's already-tested
            // codegen untouched. `reject`'s payload is coerced through the
            // same string conversion `throw`/`String(value)` use, since the
            // rejection channel (like the exception channel it shares) is
            // still string-only; `resolve` keeps the value's own type,
            // assimilating an already-Promise value exactly as a bare
            // `return somePromise` inside an executor would.
            let is_reject = callee_name == "Promise.reject";
            if !is_reject && call.args.is_empty() {
                if call
                    .type_args
                    .as_ref()
                    .is_some_and(|args| !matches!(args.params.as_slice(), [] | [_]))
                {
                    return Err("`Promise.resolve` accepts at most one type argument".into());
                }
                let resolve_ty = HirType::Function(Vec::new(), Box::new(HirType::Void));
                let executor = HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name: "__thaw_promise_resolve".into(),
                        ty: resolve_ty,
                    }],
                    HirType::Void,
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_promise_resolve".into())),
                        Vec::new(),
                    )),
                );
                return Ok(HirExpr::PromiseNew(
                    Box::new(executor),
                    HirType::Void,
                    false,
                    false,
                ));
            }
            let [argument] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if argument.spread.is_some() {
                return Err(format!(
                    "`{callee_name}` does not support a spread argument"
                ));
            }
            let mut value = self.lower_expr(&argument.expr)?;
            // Keep `Bytes`: the resolved payload type is what a `then` callback's parameter gets.
            let mut value_type = self.infer_expr_type_inner(&value)?;
            if !is_reject && matches!(value_type, HirType::Json | HirType::JsValue) {
                // An `any` argument may be a thenable: `__thaw_defer_thenable` reads its `then` once now and
                // defers the call to a microtask, as PromiseResolveThenableJob does.
                let arguments = self.coerce_to_declared(&HirType::Json, HirExpr::ArrayLit(vec![value]))?;
                let handle = HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamicMethodHandleRaw".to_string())),
                    vec![
                        HirExpr::Call(
                            Box::new(HirExpr::Var("getDynamicValue".to_string())),
                            vec![HirExpr::Lit(HirLit::Str("globalThis".to_string()))],
                        ),
                        HirExpr::Lit(HirLit::Str("__thaw_defer_thenable".to_string())),
                        arguments,
                    ],
                );
                value = self.coerce_to_declared(&HirType::Json, handle)?;
                value_type = HirType::Json;
            }
            let value_name = format!("__thaw_promise_argument_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(value_name.clone(), value_type.clone());
            let binding = (value_name.clone(), value_type.clone(), value);
            let value = HirExpr::Var(value_name);
            let (resolved, assimilates) = if is_reject {
                let resolved = match &call.type_args {
                    Some(type_args) => {
                        let [resolved] = type_args.params.as_slice() else {
                            return Err("`Promise.reject` accepts at most one type argument".into());
                        };
                        lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?
                    }
                    None => HirType::Void,
                };
                (resolved, false)
            } else {
                match &value_type {
                    HirType::Promise(inner) => (inner.as_ref().clone(), true),
                    other => (other.clone(), false),
                }
            };
            if !is_reject && assimilates {
                return self.wrap_call_argument_bindings(value, &[binding]);
            }
            let resolve_params = if assimilates {
                vec![HirType::Promise(Box::new(resolved.clone()))]
            } else if resolved == HirType::Void {
                Vec::new()
            } else {
                vec![resolved.clone()]
            };
            let resolve_ty = HirType::Function(resolve_params, Box::new(HirType::Void));
            let reject_ty = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));

            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&value, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    self.scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();

            let (params, callee) = if is_reject {
                (
                    vec![
                        HirParam {
                            name: "__thaw_promise_resolve".to_string(),
                            ty: resolve_ty,
                        },
                        HirParam {
                            name: "__thaw_promise_reject".to_string(),
                            ty: reject_ty,
                        },
                    ],
                    "__thaw_promise_reject",
                )
            } else {
                (
                    vec![HirParam {
                        name: "__thaw_promise_resolve".to_string(),
                        ty: resolve_ty,
                    }],
                    "__thaw_promise_resolve",
                )
            };
            let body = if is_reject {
                let value_name = format!("__thaw_rejected_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(value_name.clone(), value_type.clone());
                let value_var = HirExpr::Var(value_name.clone());
                let setters =
                    self.rejection_provenance_statements(value_var.clone(), &value_type)?;
                let message =
                    self.rejection_display_without_user_coercion(value_var, &value_type)?;
                let mut rejection = vec![HirStmt::Let(value_name, value_type, value)];
                rejection.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var(
                        "__thaw_clear_pending_exception_provenance".into(),
                    )),
                    Vec::new(),
                )));
                rejection.extend(setters);
                rejection.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var(callee.to_string())),
                    vec![message],
                )));
                HirExpr::Block(rejection)
            } else {
                HirExpr::Call(Box::new(HirExpr::Var(callee.to_string())), vec![value])
            };
            let executor = HirExpr::Lambda(captures, params, HirType::Void, Box::new(body));
            return self.wrap_call_argument_bindings(
                HirExpr::PromiseNew(Box::new(executor), resolved, assimilates, is_reject),
                &[binding],
            );
        }

        // `Number`/`String`/`Boolean` convert a `Json` leaf to a concrete
        // value. Unlike `console.log` (whose codegen can disambiguate its
        // argument by LLVM value shape -- f64 vs. pointer), `Str`/`Array`/
        if matches!(callee_name.as_str(), "parseFloat" | "parseInt") {
            return self.lower_parse_call(call, callee_name == "parseInt");
        }
        if callee_name == "Symbol" {
            if call.args.len() > 1 || call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err("`Symbol(...)` expects at most one non-spread argument".into());
            }
            let description = match call.args.first() {
                Some(argument) => {
                    let value = self.lower_expr(&argument.expr)?;
                    let ty = self.infer_expr_type(&value)?;
                    if matches!(ty, HirType::Undefined | HirType::Void) {
                        HirExpr::EvalThen(
                            Box::new(value),
                            Box::new(HirExpr::OptionalNone(HirType::Str)),
                        )
                    } else {
                        let name = format!("__thaw_symbol_description_input_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let bound = HirExpr::Var(name.clone());
                        let absent = self.function_observable_absence(bound.clone(), &ty);
                        let text = self.coerce_primitive_to_string(bound)?;
                        let description = HirExpr::Conditional(
                            Box::new(HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(absent),
                                Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                            )),
                            Box::new(HirExpr::OptionalNone(HirType::Str)),
                            Box::new(HirExpr::OptionalSome(Box::new(text), HirType::Str)),
                            HirType::Optional(Box::new(HirType::Str)),
                        );
                        self.wrap_call_argument_bindings(description, &[(name, ty, value)])?
                    }
                }
                None => HirExpr::OptionalNone(HirType::Str),
            };
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_symbol_new".into())),
                vec![description],
            ));
        }

        // `Object`/`Json` all share the same pointer representation, so
        // this has to be resolved here at lowering time using the
        // argument's inferred type, not deferred to codegen.
        if matches!(
            callee_name.as_str(),
            "Number" | "String" | "Boolean" | "BigInt"
        ) {
            let [arg] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if arg.spread.is_some() {
                let (arguments, bindings) =
                    self.lower_native_spread_values(&call.args, &callee_name)?;
                let [value] = arguments.as_slice() else {
                    return Err(format!("`{callee_name}` expects exactly one argument"));
                };
                return self.lower_primitive_conversion(&callee_name, value.clone(), bindings);
            }
            let value = self.lower_expr(&arg.expr)?;
            return self.lower_primitive_conversion(&callee_name, value, Vec::new());
        }

        // `Error("x")` (no `new`) behaves exactly like `new Error("x")`: a
        // tagged exception string. `new` is handled in `lower_new_expr`.
        if matches!(
            callee_name.as_str(),
            "Error"
                | "TypeError"
                | "RangeError"
                | "SyntaxError"
                | "ReferenceError"
                | "EvalError"
                | "URIError"
        ) {
            if call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err(format!(
                    "`{callee_name}()` does not support spread arguments"
                ));
            }
            if call.args.len() > 2 {
                return Err(format!(
                    "`{callee_name}()` expects at most a message and options argument"
                ));
            }
            let message = match call.args.first() {
                Some(argument) => {
                    let message = self.lower_expr(&argument.expr)?;
                    self.coerce_primitive_to_string(message)?
                }
                None => HirExpr::Lit(HirLit::Str(String::new())),
            };
            let tagged = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_frame".to_string())),
                vec![HirExpr::Lit(HirLit::Str(callee_name.clone())), message],
            );
            let Some(options) = call.args.get(1) else {
                return Ok(tagged);
            };
            let options = self.lower_expr(&options.expr)?;
            let options_type = self.infer_expr_type(&options)?;
            return self.lower_error_options(tagged, options, options_type);
        }

        let signature = self.signatures.get(&callee_name).cloned();
        let value_overloads = self.scope.get(&callee_name).and_then(|ty| match ty {
            HirType::OverloadedFunction(public, _) => Some(public.clone()),
            _ => None,
        });
        let public_overloads = signature
            .as_ref()
            .map(|signature| signature.public_overloads.as_slice())
            .filter(|overloads| !overloads.is_empty())
            .or(value_overloads.as_deref());
        let selected_public_signature = public_overloads
            .map(|overloads| {
                self.select_public_overload_for_source(
                    overloads,
                    &call.args,
                    None,
                    &callee_name,
                )
            })
            .transpose()?
            .flatten();
        let local_function = self.scope.get(&callee_name).and_then(|ty| {
            let ty = match ty {
                HirType::FunctionWithThis(_, visible) => visible.as_ref(),
                HirType::OverloadedFunction(_, physical) => physical.as_ref(),
                other => other,
            };
            match ty {
            HirType::Function(params, ret) => Some((
                params.clone(),
                ret.as_ref().clone(),
                None,
                HirOptionalMask::default(),
            )),
            HirType::CallableFunction(params, optional, rest, ret) => {
                let mut abi_params = params.clone();
                if let Some(rest) = rest {
                    abi_params.push(HirType::Array(rest.clone()));
                }
                Some((
                    abi_params,
                    ret.as_ref().clone(),
                    rest.as_deref().cloned(),
                    optional.clone(),
                ))
            }
            _ => None,
            }
        });
        let quickjs_function_value = if let Callee::Expr(callee) = &call.callee {
            self.validate_quickjs_function_value_arguments(callee, &call.args)?
        } else {
            false
        };
        let mut param_types = signature
            .as_ref()
            .map(|sig| sig.params.clone())
            .or_else(|| {
                local_function
                    .as_ref()
                    .map(|(params, _, _, _)| params.clone())
            });

        let mut argument_bindings = Vec::new();
        let mut lowered_arguments = Vec::new();
        let mut forwarded_rest_array = false;
        let mut method_argument_sources_aligned =
            call.args.iter().all(|argument| argument.spread.is_none());
        let direct_jit_typed_call = signature.as_ref().is_some_and(|signature| {
            signature.is_extern
                && method_argument_sources_aligned
                && dynamic_symbol(&callee_name)
                    .is_some_and(|(backend, _)| backend == DynamicBackend::Jit)
        });
        let mut lowered = Vec::with_capacity(call.args.len());
        let generic_placeholder = signature
            .as_ref()
            .is_some_and(|signature| !signature.generic_type_params.is_empty());
        let mut generic_inferred = HashMap::new();
        // A statically sized `...tuple` spread fills several parameter positions, so the
        // contextual parameter of a later argument is looked up at the shifted position.
        let mut spread_shift = 0usize;
        for (arg_index, argument) in call.args.iter().enumerate() {
            let index = arg_index + spread_shift;
            if argument.spread.is_none() && has_dynamic_storage(argument.expr.span()) {
                self.validate_storage_call_source(&argument.expr)?;
            }
            // Rest parameters occupy one physical array slot in the function
            // signature, while source calls supply one argument per element.
            // Use the logical element type for arguments at and after that
            // slot; `index` already includes an explicit `this` receiver when
            // `.call()` was normalized into this path.
            let rest_element = signature
                .as_ref()
                .and_then(|signature| signature.native_rest.as_ref())
                .or_else(|| {
                    local_function
                        .as_ref()
                        .and_then(|(_, _, rest, _)| rest.as_ref())
                });
            let physical_expected = param_types.as_ref().and_then(|params| {
                let rest_index = params.len().saturating_sub(1);
                match rest_element {
                    Some(element) if index >= rest_index => Some(element),
                    _ => params.get(index),
                }
            });
            let public_expected = selected_public_signature
                .as_ref()
                .and_then(|public| self.public_overload_argument_type(public, index));
            let expected = public_expected.as_ref().or(physical_expected);
            let generic_context = signature
                .as_ref()
                .filter(|_| argument.spread.is_none())
                .and_then(|signature| generic_pattern_for_physical_argument(signature, index))
                .and_then(|pattern| match pattern {
                    GenericTypePattern::Function(params, optional, rest, result) => {
                        let params = params
                            .iter()
                            .zip(optional)
                            .map(|(param, optional)| {
                                instantiate_generic_pattern(param, &generic_inferred).map(|ty| {
                                    if *optional {
                                        optional_parameter_type(ty)
                                    } else {
                                        ty
                                    }
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?;
                        let mut params = params;
                        if let Some(rest) = rest {
                            params.push(HirType::Array(Box::new(
                                instantiate_generic_pattern(rest, &generic_inferred).ok()?,
                            )));
                        }
                        Some((
                            params,
                            instantiate_generic_pattern(result, &generic_inferred).ok(),
                        ))
                    }
                    _ => None,
                });
            let mut known_actual_rest = None;
            let contextual_function = generic_context.or_else(|| {
                if argument.spread.is_some() || generic_placeholder {
                    return None;
                }
                expected.and_then(|expected| match expected {
                        HirType::Function(params, ret) => {
                            Some((params.clone(), Some(ret.as_ref().clone())))
                        }
                        HirType::CallableFunction(params, _, rest, ret) => {
                            let mut abi_params = params.clone();
                            if let Some(rest) = rest {
                                let (argument_count, declares_rest, actual_has_rest) =
                                    match argument.expr.as_ref() {
                                    Expr::Arrow(arrow) => (
                                        arrow.params.len(),
                                        matches!(arrow.params.last(), Some(Pat::Rest(_))),
                                        false,
                                    ),
                                    Expr::Fn(function) => (
                                        function.function.params.len(),
                                        matches!(
                                            function.function.params.last(),
                                            Some(param) if matches!(param.pat, Pat::Rest(_))
                                        ),
                                        false,
                                    ),
                                    Expr::Ident(ident) => {
                                        let name = self.resolve_binding(ident.sym.as_ref());
                                            let generic_template =
                                                self.generic_named_templates.contains_key(&name)
                                            || self.generic_arrows.contains_key(&name);
                                        let actual = if generic_template {
                                            None
                                        } else if let Some(ty) = self.scope.get(&name) {
                                            let wrapped_actual = matches!(
                                                ty,
                                                HirType::FunctionWithThis(_, _)
                                                    | HirType::Optional(_)
                                                    | HirType::Nullable(_)
                                                    | HirType::Nullish(_)
                                            );
                                            let mut ty = ty;
                                            loop {
                                                ty = match ty {
                                                    HirType::FunctionWithThis(_, visible)
                                                    | HirType::Optional(visible)
                                                    | HirType::Nullable(visible)
                                                    | HirType::Nullish(visible) => visible,
                                                    _ => break,
                                                };
                                            }
                                            match ty {
                                                HirType::Function(params, _) => {
                                                    Some((params.len(), false, false))
                                                }
                                                    HirType::CallableFunction(
                                                        params,
                                                        _,
                                                        actual_rest,
                                                        _,
                                                    ) => Some((
                                                        params.len()
                                                            + usize::from(actual_rest.is_some()),
                                                        actual_rest.is_some(),
                                                        actual_rest.is_some() && !wrapped_actual,
                                                    )),
                                                _ => None,
                                            }
                                        } else {
                                                self.signatures
                                                    .get(&name)
                                                    .filter(|signature| {
                                                        signature.generic_type_params.is_empty()
                                                    })
                                                    .map(|signature| {
                                                        (
                                                    signature.params.len(),
                                                    signature.native_rest.is_some(),
                                                    signature.native_rest.is_some(),
                                                        )
                                                    })
                                        };
                                        if generic_template {
                                            (abi_params.len() + 1, true, false)
                                        } else if let Some(shape) = actual {
                                            shape
                                        } else {
                                            (abi_params.len() + 1, true, false)
                                        }
                                    }
                                    _ => (abi_params.len() + 1, true, false),
                                };
                                if declares_rest {
                                    abi_params.resize(
                                        argument_count.saturating_sub(1).max(abi_params.len()),
                                        rest.as_ref().clone(),
                                    );
                                    abi_params.push(HirType::Array(rest.clone()));
                                } else {
                                    abi_params.resize(
                                        argument_count.max(abi_params.len()),
                                        rest.as_ref().clone(),
                                    );
                                }
                                if actual_has_rest {
                                    let fixed_prefix = abi_params.len().saturating_sub(1);
                                    known_actual_rest = Some((
                                        abi_params.clone(),
                                        fixed_prefix,
                                        rest.as_ref().clone(),
                                    ));
                                }
                            }
                            Some((abi_params, Some(ret.as_ref().clone())))
                        }
                        _ => None,
                })
            });
            let spread_expected = (index..index + call.args.len() + 1)
                .filter_map(|argument_index| {
                    selected_public_signature
                        .as_ref()
                        .and_then(|public| {
                            self.public_overload_argument_type(public, argument_index)
                        })
                        .or_else(|| {
                            param_types.as_ref().and_then(|params| {
                                let rest_index = params.len().saturating_sub(1);
                                match rest_element {
                                    Some(element) if argument_index >= rest_index => {
                                        Some(element.clone())
                                    }
                                    _ => params.get(argument_index).cloned(),
                                }
                            })
                        })
                })
                .collect::<Vec<_>>();
            // An ambient QuickJS export passed by bare name to a generic or dynamic
            // parameter (`promisify(execFile)`) is the package's real JS function
            // (see `ambient_js_export_argument`), not the typed stub.
            let ambient_value = if matches!(argument.expr.as_ref(), Expr::Ident(_))
                && (!generic_placeholder)
                    .then_some(expected)
                    .flatten()
                    .is_none_or(|ty| matches!(ty, HirType::Dynamic | HirType::Json | HirType::JsValue))
            {
                self.ambient_js_export_argument(&argument.expr, argument.spread.is_some())
            } else {
                None
            };
            let mut value = if let Some(value) = ambient_value {
                value
            } else if let Some(value) = self.lower_literal_call_spread(
                argument,
                &spread_expected,
                rest_element,
                0,
            )? {
                value
            } else if let Some((params, ret)) = contextual_function {
                if matches!(
                    argument.expr.as_ref(),
                    Expr::Arrow(_) | Expr::Fn(_) | Expr::Ident(_)
                ) {
                    self.lower_promise_callback(&argument.expr, &params, ret.as_ref())?
                } else {
                    self.lower_expr(&argument.expr)?
                }
            } else {
                let expected = (!generic_placeholder).then_some(expected).flatten();
                // No declared slot to type the callback from (an untyped callee): its
                // parameters are contextually `any`.
                let implicit_any = if expected.is_none() {
                    self.lower_implicit_any_callback(&argument.expr)
                } else {
                    None
                };
                match argument.expr.as_ref() {
                    _ if implicit_any.is_some() => implicit_any.expect("checked")?,
                    Expr::Arrow(arrow)
                        if signature
                            .as_ref()
                            .is_some_and(|signature| signature.is_extern)
                            && matches!(
                                expected,
                                Some(HirType::Dynamic | HirType::Json | HirType::JsValue)
                            ) =>
                    {
                        self.lower_contextual_arrow(
                            arrow,
                            &vec![HirType::JsValue; arrow.params.len()],
                            None,
                        )?
                    }
                    // An argument passed into an untyped `any`/`Json`/
                    // `JsValue` slot of a Fallback package function gets
                    // the same one-shot `JsValue` hint a dynamic method
                    // call's own arguments already get (`lower_dynamic_
                    // value_method_call`), so a chained call in that
                    // position (real example: rxjs's own `firstValueFrom
                    // (of(7).pipe(delay(20)))`, whose `source` parameter is
                    // `Observable<T>` -> `Json`) keeps its live handle
                    // instead of silently defaulting to a content-free JSON
                    // snapshot (`{}`).
                    Expr::Call(_)
                        if matches!(
                            expected,
                            Some(HirType::Json | HirType::JsValue | HirType::Dynamic)
                        ) =>
                    {
                        self.lower_expr_with_expected_type(&argument.expr, Some(&HirType::JsValue))?
                    }
                    _ => {
                        let console_read = if callee_name.starts_with("console.") {
                            match self.lower_console_dictionary_read(&argument.expr)? {
                                Some(read) => Some(read),
                                None => self.lower_string_index_optional(&argument.expr)?,
                            }
                        } else {
                            None
                        };
                        match console_read {
                            Some(read) => read,
                            None if argument.spread.is_none()
                                && method_argument_sources_aligned
                                && expected.is_some_and(|ty| matches!(ty, HirType::Object(_)) && class_name_from_type(ty).is_none())
                                && signature.as_ref().is_some_and(|signature| signature.is_extern
                                    && dynamic_symbol(&callee_name).is_some_and(|(backend, symbol)|
                                        backend == DynamicBackend::Jit
                                            || Self::is_ordinary_quickjs_typed_call(backend, &symbol)))
                                && self.source_contract_type_for_expression(&argument.expr)
                                    .is_some_and(|ty| class_name_from_type(&ty).is_some()) =>
                            {
                                self.validate_storage_source(&argument.expr, expected.expect("guarded expected type"))?;
                                // Keep the physical class value for the live projection after source-shape validation.
                                self.lower_expr(&argument.expr)?
                            }
                            None => self.lower_expr_with_expected_type(&argument.expr, expected)?,
                        }
                    }
                }
            };
            value = self.lower_storage_call_spread(argument, value)?;
            // `f(xs[0])` for a generic `f` with a method-bearing element: the read only looks
            // optional, so `T` must be the element itself.
            if argument.spread.is_none()
                && signature.as_ref().is_some_and(|signature| !signature.generic_type_params.is_empty())
                && matches!(argument.expr.as_ref(), Expr::Member(member) if matches!(member.prop, MemberProp::Computed(_)))
            {
                if let Ok(HirType::Optional(payload)) = self.infer_expr_type(&value) {
                    if matches!(payload.as_ref(), HirType::Object(fields)
                        if class_name_from_type(&payload).is_some()
                            || fields.iter().any(|(_, ty)| matches!(ty, HirType::Function(..) | HirType::CallableFunction(..))))
                    {
                        let (unwrapped, _) = self.unwrap_required_optional_member(
                            value, HirType::Optional(payload), "argument", false,
                        )?;
                        value = unwrapped;
                    }
                }
            }
            if let Some((physical_params, fixed_prefix, rest_element)) = known_actual_rest.take() {
                if physical_params.len() != fixed_prefix + 1
                    || physical_params.get(fixed_prefix)
                        != Some(&HirType::Array(Box::new(rest_element.clone())) )
                {
                    return Err(
                        "internal contextual rest callback ABI has an invalid tail boundary".into(),
                    );
                }
                value = match self.infer_expr_type(&value)? {
                    HirType::Function(params, ret) if params == physical_params => {
                        HirExpr::TypedClosure(
                            HirType::CallableFunction(
                                physical_params[..fixed_prefix].to_vec(),
                                HirOptionalMask::default(),
                                Some(Box::new(rest_element)),
                                ret,
                            ),
                            Box::new(value),
                        )
                    }
                    HirType::CallableFunction(params, _, Some(actual_rest), _)
                        if params.as_slice() == &physical_params[..fixed_prefix]
                            && actual_rest.as_ref() == &rest_element =>
                    {
                        value
                    }
                    actual => {
                        return Err(format!(
                            "normalized named rest callback has type {actual:?}, expected physical parameters {physical_params:?}"
                        ));
                    }
                };
            }
            if let Some(pattern) = signature
                .as_ref()
                .and_then(|signature| generic_pattern_for_physical_argument(signature, index))
            {
                let source = argument
                    .spread
                    .is_none()
                    .then_some(argument.expr.as_ref());
                let actual = self.generic_call_argument_source_type(
                    source,
                    &value,
                    signature.as_ref().is_some_and(|signature| !signature.is_extern),
                )?;
                let _ = match_generic_pattern(pattern, &actual, &mut generic_inferred);
            }
            if argument.spread.is_some() {
                let width = match &value {
                    HirExpr::ArrayLit(elements) => Some(elements.len()),
                    other => match self.infer_expr_type(other) {
                        Ok(HirType::Tuple(elements)) => Some(elements.len()),
                        _ => None,
                    },
                };
                if let Some(width) = width {
                    spread_shift += width.saturating_sub(1);
                }
            }
            lowered.push(value);
        }
        let preserve_argument_order = class_method_guard.is_some()
            || call.args.iter().any(|arg| arg.spread.is_some())
            || lowered.iter().any(contains_await);
        for (arg, mut value) in call.args.iter().zip(lowered) {
            if !preserve_argument_order {
                lowered_arguments.push(value);
                continue;
            }
            if arg.spread.is_none() {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_call_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                // The temporary stands for the same physical value, so a class
                // instance viewed through an interface keeps its class alias
                // (`const n: Named = new C(); n.method()` passes `n` as the
                // class receiver).
                if let HirExpr::Var(source) = &value {
                    if let Some(alias) = self.native_class_aliases.get(source).cloned() {
                        self.native_class_aliases.insert(name.clone(), alias);
                    }
                }
                argument_bindings.push((name.clone(), ty, value));
                lowered_arguments.push(HirExpr::Var(name));
                continue;
            }

            if let HirExpr::ArrayLit(values) = value {
                for value in values {
                    let ty = self.infer_expr_type(&value)?;
                    let name = format!("__thaw_call_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    argument_bindings.push((name.clone(), ty, value));
                    lowered_arguments.push(HirExpr::Var(name));
                }
                continue;
            }

            value = self.project_native_base(value)?;
            value = self.bytes_to_f64_array(value)?;
            let mut source_type = self.infer_expr_type(&value)?;
            let rest_element = signature
                .as_ref()
                .and_then(|signature| signature.native_rest.as_ref())
                .or_else(|| {
                    local_function
                        .as_ref()
                        .and_then(|(_, _, rest, _)| rest.as_ref())
                });
            let fixed_count = param_types.as_ref().map_or(0, |params| {
                params.len() - usize::from(rest_element.is_some())
            });
            if lowered_arguments.len() == fixed_count {
                if let (HirType::Union(members), Some(rest_element)) = (&source_type, rest_element)
                {
                    if members
                        .iter()
                        .all(|member| matches!(member, HirType::Array(_)))
                {
                        let (flattened, flattened_type) =
                            self.lower_union_array_sequence(value.clone(), members)?;
                        if rest_element == &HirType::Dynamic {
                            value = flattened;
                            source_type = flattened_type;
                        } else {
                            let expected_array = HirType::Array(Box::new(rest_element.clone()));
                            if let Ok(converted) =
                                self.coerce_to_declared(&expected_array, flattened)
                            {
                                value = converted;
                                source_type = expected_array;
                            }
                        }
                    }
                }
            }
            if let (HirType::Array(element), Some(rest_element)) = (&source_type, rest_element) {
                if lowered_arguments.len() == fixed_count
                    && (element.as_ref() == rest_element || rest_element == &HirType::Dynamic)
                {
                    lowered_arguments.push(value);
                    forwarded_rest_array = true;
                    continue;
                }
            }
            let HirType::Tuple(elements) = &source_type else {
                return Err(format!(
                    "call spread source must have statically known tuple length, got {source_type:?}"
                ));
            };
            let elements = elements.clone();
            let name = format!("__thaw_call_spread_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), source_type.clone());
            argument_bindings.push((name.clone(), source_type, value));
            lowered_arguments.extend(elements.into_iter().enumerate().map(|(index, element)| {
                HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    element,
                )
            }));
        }

        let selected_public_return = public_overloads
            .map(|overloads| {
                self.select_public_overload(overloads, &lowered_arguments, None, &callee_name)
            })
            .transpose()?
            .flatten();

        // A selected public overload can expose its rest parameter before
        // implementation-only optional slots. Preserve source order, but
        // insert those omitted fixed ABI values before the public rest tail.
        if let Some(public) = selected_public_signature.as_ref() {
            let public_has_rest = match public {
                HirType::FunctionWithThis(_, callable) => matches!(callable.as_ref(),
                    HirType::CallableFunction(_, _, Some(_), _)),
                HirType::CallableFunction(_, _, Some(_), _) => true,
                _ => false,
            };
            let implementation_rest = signature.as_ref().and_then(|signature| {
                signature.native_rest.as_ref().map(|_| (signature.params.as_slice(), signature.params.len().saturating_sub(1)))
            }).or_else(|| local_function.as_ref().and_then(|(params, _, rest, _)| {
                rest.as_ref().map(|_| (params.as_slice(), params.len().saturating_sub(1)))
            }));
            if let Some((implementation_params, implementation_fixed_count)) = implementation_rest {
                    let (public_fixed_count, public_optional) = match public {
                        HirType::FunctionWithThis(_, callable) => match callable.as_ref() {
                            HirType::CallableFunction(params, optional, _, _) => (params.len(), optional.clone()),
                            HirType::Function(params, _) => (params.len(), HirOptionalMask::default()),
                            _ => (0, HirOptionalMask::default()),
                        },
                        HirType::CallableFunction(params, optional, _, _) => (params.len(), optional.clone()),
                        HirType::Function(params, _) => (params.len(), HirOptionalMask::default()),
                        _ => (0, HirOptionalMask::default()),
                    };
                    if public_has_rest && public_fixed_count < implementation_fixed_count
                        && lowered_arguments.len() > public_fixed_count {
                        let public_rest_values = lowered_arguments.split_off(public_fixed_count);
                        for parameter in implementation_params.iter().take(implementation_fixed_count).skip(public_fixed_count) {
                            lowered_arguments.push(omitted_parameter_value(parameter)?);
                        }
                        lowered_arguments.extend(public_rest_values);
                    } else if !public_has_rest && lowered_arguments.len() < public_fixed_count {
                        for (index, parameter) in implementation_params.iter().enumerate()
                            .take(public_fixed_count.min(implementation_fixed_count))
                            .skip(lowered_arguments.len()) {
                            if !public_optional.contains(index) {
                                break;
                            }
                            lowered_arguments.push(omitted_parameter_value(parameter)?);
                        }
                    }
            }
        }

        let fixed_arguments_before_native_rest_pack = lowered_arguments.len();
        let native_rest_values = if forwarded_rest_array {
            None
        } else {
            signature.as_ref().and_then(|signature| {
                signature.native_rest.clone().map(|element| {
                let fixed_count = signature.params.len() - 1;
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
                })
            })
        };
        let packed_native_rest_fixed_count = native_rest_values.as_ref().and_then(|_| {
            signature.as_ref().and_then(|signature| {
                let rest_index = signature.params.len().checked_sub(1)?;
                Some(fixed_arguments_before_native_rest_pack.min(rest_index))
            })
        });
        let local_rest_values = if forwarded_rest_array {
            None
        } else {
            local_function
                .as_ref()
                .and_then(|(_, _, rest, _)| rest.clone())
                .map(|element| {
                let fixed_count = param_types
                    .as_ref()
                    .map_or(0, |params| params.len().saturating_sub(1));
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
            })
        };

        if let Some((fixed, _, _, optional)) = &local_function {
            let fixed_count = fixed.len() - usize::from(local_rest_values.is_some());
            if lowered_arguments.len() < fixed_count {
                for (index, parameter) in fixed
                    .iter()
                    .enumerate()
                    .take(fixed_count)
                    .skip(lowered_arguments.len())
                {
                    let public_optional = selected_public_signature
                        .as_ref()
                        .is_some_and(|public| {
                            let callable = match public {
                                HirType::FunctionWithThis(_, callable) => callable.as_ref(),
                                callable => callable,
                            };
                            match callable {
                                HirType::CallableFunction(_, public_optional, _, _) => {
                                    public_optional.contains(index)
                                }
                                _ => false,
                            }
                        });
                    if !is_optional_parameter(optional, index) && !public_optional {
                        return Err(format!(
                            "function `{callee_name}` expects argument {}, but it was omitted",
                            index + 1
                        ));
                    }
                    lowered_arguments.push(omitted_parameter_value(parameter)?);
                }
            }
        }

        // Native rest arguments are packed into one ABI slot below. Fill
        // omitted trailing fixed parameters first, while the rest values are
        // still separate, so the packed array stays in its declared slot.
        if native_rest_values.is_some() {
            if let Some(signature) = signature
                .as_ref()
                .filter(|signature| signature.generic_type_params.is_empty())
            {
                let fixed_count = signature.params.len().saturating_sub(1);
                for (index, ty) in signature
                    .params
                    .iter()
                    .enumerate()
                    .take(fixed_count)
                    .skip(lowered_arguments.len())
                {
                    if signature.source_omittable.contains(index) {
                        lowered_arguments.push(omitted_parameter_value(ty)?);
                    } else if matches!(ty, HirType::Json | HirType::Dynamic | HirType::JsValue) {
                        lowered_arguments.push(HirExpr::Lit(HirLit::Undefined));
                    } else {
                        break;
                    }
                }
            }
        }

        if let Some(signature) = signature.as_ref() {
            for (index, argument) in lowered_arguments.iter_mut().enumerate() {
                if signature.source_omittable.contains(index)
                    && self.infer_expr_type(argument)? == HirType::Undefined
                {
                    *argument = omitted_parameter_value(&signature.params[index])?;
                }
            }
        }

        if let Some((element, values)) = native_rest_values {
            method_argument_sources_aligned = false;
            let packed = if element == HirType::Dynamic {
                let expected = signature
                    .as_ref()
                    .and_then(|signature| signature.params.last())
                    .ok_or("tuple rest parameter is missing its ABI slot")?;
                self.coerce_to_declared(expected, HirExpr::ArrayLit(values))?
            } else {
                let values = values
                    .into_iter()
                    .map(|value| self.coerce_to_declared(&element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                native_rest_array(values, &element)
            };
            lowered_arguments.push(packed);
            param_types = signature.as_ref().map(|signature| signature.params.clone());
        }
        if let Some((element, values)) = local_rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(&element, value))
                .collect::<Result<Vec<_>, _>>()?;
            lowered_arguments.push(native_rest_array(values, &element));
        }

        if let Some(params) = &param_types {
            // JavaScript accepts fewer arguments than parameters (a missing
            // one is `undefined`). Pad the missing trailing parameters that
            // can hold `undefined` (`any`/optional) so a shorter call matches
            // -- e.g. test262's `verifyProperty(obj, name, desc, strict)`
            // called with three. A concretely typed parameter still requires
            // its argument.
            if signature.as_ref().is_some_and(|sig| {
                sig.generic_type_params.is_empty()
                    && sig.variadic.is_none()
                    && sig.native_rest.is_none()
            }) {
                let signature = signature.as_ref().expect("checked above");
                for (index, ty) in params
                    .iter()
                    .enumerate()
                    .skip(lowered_arguments.len().min(params.len()))
                {
                    if signature.source_omittable.contains(index) {
                        lowered_arguments.push(omitted_parameter_value(ty)?);
                    } else if matches!(ty, HirType::Json | HirType::Dynamic | HirType::JsValue) {
                        lowered_arguments.push(HirExpr::Lit(HirLit::Undefined));
                    } else {
                        break;
                    }
                }
            }
            let variadic = signature.as_ref().and_then(|sig| sig.variadic.as_ref());
            let generic_optional = signature.as_ref().filter(|signature| {
                !signature.generic_type_params.is_empty()
                    && signature
                        .generic_param_optional
                        .iter()
                        .any(|optional| *optional)
            });
            let wrong_count = if let Some(signature) = generic_optional {
                let required = signature
                    .generic_param_optional
                    .iter()
                    .take_while(|optional| !**optional)
                    .count()
                    + usize::from(signature.uses_this);
                lowered_arguments.len() < required || lowered_arguments.len() > params.len()
            } else if variadic.is_some() {
                lowered_arguments.len() < params.len()
            } else {
                lowered_arguments.len() != params.len()
            };
            if wrong_count {
                return Err(format!(
                    "function `{callee_name}` expects {}{} argument(s), got {}",
                    if variadic.is_some() { "at least " } else { "" },
                    params.len(),
                    lowered_arguments.len()
                ));
            }
        }

        let direct_quickjs_typed_call = signature.as_ref().is_some_and(|signature| {
            signature.is_extern
                && dynamic_symbol(&callee_name).is_some_and(|(backend, symbol)| {
                    Self::is_ordinary_quickjs_typed_call(backend, &symbol)
                })
        });
        let quickjs_instance_typed_method = signature.as_ref().is_some_and(|signature| {
            signature.is_extern
                && method_argument_sources_aligned
                && dynamic_symbol(&callee_name).is_some_and(|(backend, symbol)| {
                    Self::is_quickjs_instance_typed_method(backend, &symbol)
                })
        });
        let napi_constructor_typed_call = signature.as_ref().is_some_and(|signature| {
            signature.is_extern
                && dynamic_symbol(&callee_name).is_some_and(|(backend, symbol)| {
                    Self::is_napi_constructor_typed_call(backend, &symbol, &signature.ret)
                })
        });
        let mut args = lowered_arguments
            .into_iter()
            .enumerate()
            .map(|(i, value)| -> Result<HirExpr, String> {
                match param_types
                    .as_ref()
                    .and_then(|p| p.get(i))
                    .or_else(|| signature.as_ref().and_then(|sig| sig.variadic.as_ref()))
                {
                    Some(_)
                        if signature
                            .as_ref()
                            .is_some_and(|sig| !sig.generic_type_params.is_empty()) =>
                    {
                        Ok(value)
                    }
                    Some(declared) => {
                        let public_expected = selected_public_signature
                            .as_ref()
                            .and_then(|public| self.public_overload_argument_type(public, i));
                        let value = self.adapt_selected_overload_argument(
                            declared,
                            public_expected.as_ref(),
                            value,
                        )?;
                        let original_is_jsvalue = self.infer_expr_type(&value)? == HirType::JsValue;
                        let actual = self.infer_expr_type(&value)?;
                        let live_method_json_argument = quickjs_instance_typed_method
                            && i > 0
                            && actual == HirType::Json
                            && call
                                .args
                                .get(i)
                                .is_some_and(|argument| has_dynamic_storage(argument.expr.span()));
                        if live_method_json_argument {
                            self.validate_storage_source(&call.args[i].expr, declared)?;
                        }
                        let class_object =
                            direct_quickjs_typed_call && class_name_from_type(&actual).is_some();
                        let jit_structural_class_object = direct_jit_typed_call
                            && class_name_from_type(&actual).is_some()
                            && matches!(declared, HirType::Object(_))
                            && class_name_from_type(declared).is_none();
                        let is_generated_wrapper =
                            callee_name.starts_with("__thaw_typed_wrapper_js_");
                        let user_function = !(is_generated_wrapper
                            || direct_quickjs_typed_call
                            || napi_constructor_typed_call)
                            && signature.as_ref().is_some_and(|sig| !sig.is_extern);
                        let promoted_typed_callback = matches!(value, HirExpr::FunctionRef(..))
                            || direct_quickjs_typed_call
                                && matches!(
                                    value,
                                    HirExpr::Var(_)
                                        | HirExpr::Lambda(..)
                                        | HirExpr::TypedClosure(..)
                                );
                        let promoted_typed_callback = promoted_typed_callback
                            && typed_callback_storage_return_matches(&actual, declared);
                        if promoted_typed_callback
                            && matches!(value, HirExpr::FunctionRef(..) | HirExpr::Var(_))
                        {
                            let (HirType::Function(params, ret)
                            | HirType::CallableFunction(params, _, _, ret)) = declared
                            else {
                                unreachable!()
                            };
                            self.validate_promise_callback_value(&value, params, Some(ret))
                                .map_err(|error| {
                                    format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                                })?;
                        }
                        let value = if live_method_json_argument
                            || promoted_typed_callback
                            || direct_quickjs_typed_call && actual == HirType::Json
                        {
                            Ok(value)
                        } else if class_object {
                            self.coerce_primitive_array_argument_aliasing(value, &HirType::Json)
                        } else if jit_structural_class_object {
                            self.coerce_jit_structural_class_argument(value, declared)
                        } else if quickjs_function_value || user_function {
                            self.coerce_primitive_array_argument_aliasing(value, declared)
                        } else if signature.as_ref().is_some_and(|sig| sig.is_extern)
                            && dynamic_symbol(&callee_name).is_none()
                        {
                            self.coerce_native_ffi_argument(value, declared)
                        } else {
                            self.coerce_primitive_array_argument(value, declared)
                        }
                        .map_err(|error| {
                            format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                        })?;
                        if (is_generated_wrapper
                            || direct_quickjs_typed_call
                            || napi_constructor_typed_call)
                            && *declared == HirType::Json
                            && original_is_jsvalue
                            && Self::contains_live_host_jsvalue_json_conversion(&value)
                        {
                            let owner_kind = if is_generated_wrapper {
                                "wrapper"
                            } else if direct_quickjs_typed_call {
                                "direct-quickjs"
                            } else {
                                "napi-constructor"
                            };
                            let owner_key = if is_generated_wrapper {
                                callee_name.clone()
                            } else {
                                dynamic_symbol(&callee_name)
                                    .map(|(_, symbol)| symbol)
                                    .ok_or_else(|| {
                                        format!(
                                            "internal typed-call owner missing for `{callee_name}`"
                                        )
                                    })?
                            };
                            Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    "__thaw_mark_typed_wrapper_json_argument".into(),
                                )),
                                vec![
                                    HirExpr::Lit(HirLit::Str(owner_kind.into())),
                                    HirExpr::Lit(HirLit::Str(owner_key)),
                                    value,
                                ],
                            ))
                        } else {
                            Ok(value)
                        }
                    }
                    None => Ok(value),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        if matches!(
            callee_name.as_str(),
            "console.log"
                | "console.info"
                | "console.debug"
                | "console.warn"
                | "console.error"
                | "console.assert"
                | "console.trace"
                | "console.group"
                | "console.groupCollapsed"
                | "console.dirxml"
        ) {
            args = args.into_iter().map(|arg| match arg {
                HirExpr::TypedIndex(array, index, element)
                    if matches!(self.infer_expr_type(&array), Ok(HirType::Array(_))) => {
                        self.lower_array_index(
                            *array,
                            HirType::Array(Box::new(element.clone())),
                            element,
                            *index,
                        )
                    }
                // `Bytes` is erased to `Array(F64)` after lowering; tag it so the console
                // prints `<Buffer ..>` like Node instead of a number array.
                other if matches!(self.infer_expr_type_inner(&other), Ok(HirType::Bytes)) => Ok(
                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_console_bytes".into())), vec![other]),
                ),
                // An object with a `Buffer` field (erased to `number[]` in the typed layout):
                // rebuild it as `Json` with the field branded as a Buffer so it prints
                // `{ b: <Buffer ..> }`.
                // `{ b }` with a `Buffer` field lowers to `(param) => ({ b: param })` applied to the
                // value (evaluation order); rebuild the body as `Json` with that field branded.
                HirExpr::Call(callee, call_args) if matches!(callee.as_ref(),
                    HirExpr::Lambda(_, params, _, body) if matches!(body.as_ref(), HirExpr::ObjectLit(_))
                        && params.iter().any(|param| param.ty == HirType::Bytes)) => {
                    let HirExpr::Lambda(captures, params, _, body) = *callee else { unreachable!() };
                    let HirExpr::ObjectLit(fields) = *body else { unreachable!() };
                    let entries = fields.into_iter()
                        .map(|(name, value)| Ok((name, self.coerce_to_declared(&HirType::Json, value)?)))
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(HirExpr::Call(
                        Box::new(HirExpr::Lambda(captures, params, HirType::Json,
                            Box::new(HirExpr::JsonObjectLit(entries, HirType::Json)))),
                        call_args,
                    ))
                }
                // Any other computed value: carry HIR's inferred type to codegen, whose own type
                // mirror (`expr_hir_type`) cannot see through every call/builtin result.
                other if !matches!(&other, HirExpr::Var(_) | HirExpr::Lit(_))
                    && !matches!(&other, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name.starts_with("__thaw_console_")))
                    && matches!(self.infer_expr_type_inner(&other), Ok(HirType::Array(_) | HirType::Object(_) | HirType::Tuple(_) | HirType::Map(..) | HirType::Set(_) | HirType::Dictionary(_))) => {
                    let ty = self.infer_expr_type_inner(&other)?;
                    Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_console_typed".into())),
                        vec![HirExpr::TypedClosure(ty, Box::new(other))]))
                }
                other => Ok(other),
            }).collect::<Result<Vec<_>, _>>()?;
        }

        match callee_name.as_str() {
            // A label argument: always a string (`"default"` when omitted).
            "console.count" | "console.countReset" | "console.time" | "console.timeEnd"
            | "console.timeLog" => {
                let label = if args.is_empty() {
                    HirExpr::Lit(HirLit::Str("default".into()))
                } else {
                    let first = args.remove(0);
                    self.coerce_primitive_to_string(first)?
                };
                args = vec![label];
            }
            // `console.table(data, columns?)` / `console.dir(value, options?)` take `any` values.
            "console.table" | "console.dir" => {
                args.truncate(2);
                args = args
                    .into_iter()
                    .map(|arg| self.coerce_to_declared(&HirType::Json, arg))
                    .collect::<Result<Vec<_>, String>>()?;
            }
            _ => {}
        }

        if callee_name == "console.assert" {
            if args.is_empty() {
                args.push(HirExpr::Lit(HirLit::Bool(false)));
            } else {
                let condition_value = args.remove(0);
                let ty = self.infer_expr_type(&condition_value)?;
                let name = format!("__thaw_console_assert_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let condition = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
                argument_bindings.push((name, ty, condition_value));
                args.insert(0, condition);
            }
        }

        let generic_types = self.infer_generic_call_types(
            &callee_name,
            signature.as_ref(),
            &args,
            &GenericCallContext {
                call,
                packed_native_rest_fixed_count,
                expected_source_return_hint: expected_source_return_hint.as_ref(),
                expected_return_hint: expected_return_hint.as_ref(),
            },
        )?;

        if let (Some(signature), Some(types)) = (&signature, &generic_types) {
            self.retype_packed_empty_generic_rest(
                &mut args, signature, types, packed_native_rest_fixed_count,
            )?;
        }

        if let (Some(signature), Some(constraints)) = (&signature, self.call_constraints) {
            if let Some(types) = &generic_types {
                constraints
                    .borrow_mut()
                    .push(CallConstraint::Generic(callee_name.clone(), types.clone()));
            } else {
                for (index, (declared, value)) in signature.params.iter().zip(&args).enumerate() {
                    let actual = self.infer_expr_type(value)?;
                    if *declared == HirType::Dynamic
                        || (*declared == HirType::Json && actual == HirType::JsValue)
                    {
                        constraints.borrow_mut().push(CallConstraint::Parameter(
                            callee_name.clone(),
                            index,
                            actual,
                            (call.span.lo.0, call.span.hi.0),
                        ));
                    }
                }
            }
        }

        if let Some(sig) = signature.clone().filter(|sig| sig.is_extern) {
            if let Some((backend, symbol)) = dynamic_symbol(&callee_name) {
                let (params, ret) = if let Some(types) = &generic_types {
                    let substitution = sig
                        .generic_type_params
                        .iter()
                        .cloned()
                        .zip(types.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    let params = sig
                        .generic_param_patterns
                        .iter()
                        .zip(&sig.generic_param_optional)
                        .map(|(pattern, optional)| {
                            let ty = instantiate_generic_pattern(pattern, &substitution)?;
                            Ok(if *optional {
                                optional_parameter_type(ty)
                            } else {
                                ty
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let ret = resolve_ts_type_with_substitution(
                        sig.generic_return_type
                            .as_ref()
                            .expect("generic function return type"),
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )?;
                    (params, ret)
                } else {
                    (sig.params, sig.ret)
                };
                let direct_quickjs_typed_call =
                    Self::is_ordinary_quickjs_typed_call(backend, &symbol);
                let direct_jit_typed_call =
                    backend == DynamicBackend::Jit && method_argument_sources_aligned;
                let quickjs_instance_typed_method = method_argument_sources_aligned
                    && Self::is_quickjs_instance_typed_method(backend, &symbol);
                let ret = if direct_quickjs_typed_call
                    && (generic_types.is_some() || matches!(&ret, HirType::Union(_)))
                {
                    storage_type(call.span, ret)
                } else {
                    ret
                };
                let napi_constructor_typed_call =
                    Self::is_napi_constructor_typed_call(backend, &symbol, &ret);
                // Argument coercion was skipped for every generic
                // function's call above (`param_types` there was still
                // the pre-substitution, `Dynamic`-placeholder version --
                // coercing against it would be meaningless), deferred to
                // here where `params` is finally the real, substituted
                // type. Needed for a value going into a parameter that's
                // optional only *after* substitution (e.g. `nanoid`'s
                // `size?: number`, `Optional(F64)` -- a raw `5.0` needs
                // wrapping into that Optional's own representation, not
                // just passing through unwrapped). Coerces only the
                // arguments actually present, up to `params.len()` --
                // never more (a variadic call's own trailing rest
                // arguments, beyond `params.len()`, pass through
                // unchanged instead of being truncated away), and never
                // fewer either (an optional trailing parameter, like
                // `size?` above, can legitimately be omitted; arity is
                // somebody else's job, not this coercion step's).
                // `fs.readSync(fd, buf, offset, ..)` declares `buf: any`, so a
                // native Buffer reaches QuickJS as a copy and the bytes read
                // never come back. Keep the (side-effect free) buffer and
                // offset operands to copy them back after the call.
                let read_sync_writeback = if symbol == "node:fs::readSync"
                    && call.args.len() == 5
                    && call.args.iter().all(|argument| argument.spread.is_none())
                    && matches!(call.args[1].expr.as_ref(), Expr::Ident(_))
                    && matches!(call.args[2].expr.as_ref(), Expr::Ident(_) | Expr::Lit(_))
                {
                    let buffer = self.lower_expr(&call.args[1].expr)?;
                    if self.infer_expr_type_inner(&buffer)? == HirType::Bytes {
                        Some((buffer, self.lower_expr(&call.args[2].expr)?))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let fixed_count = params.len().min(args.len());
                let mut args = args.into_iter();
                let mut coerced_args = params
                    .iter()
                    .take(fixed_count)
                    .enumerate()
                    .map(|(i, declared)| -> Result<HirExpr, String> {
                        let value = args.next().expect("fixed_count <= args.len()");
                        let original_is_jsvalue = self.infer_expr_type(&value)? == HirType::JsValue;
                        let actual = self.infer_expr_type(&value)?;
                        let live_method_json_argument = quickjs_instance_typed_method
                            && i > 0
                            && actual == HirType::Json
                            && call.args.get(i).is_some_and(|argument| {
                                argument.spread.is_none()
                                    && has_dynamic_storage(argument.expr.span())
                            });
                        if live_method_json_argument {
                            self.validate_storage_source(&call.args[i].expr, declared)?;
                        }
                        let class_object =
                            direct_quickjs_typed_call && class_name_from_type(&actual).is_some();
                        let jit_structural_class_object = direct_jit_typed_call
                            && class_name_from_type(&actual).is_some()
                            && matches!(declared, HirType::Object(_))
                            && class_name_from_type(declared).is_none();
                        if jit_structural_class_object && method_argument_sources_aligned {
                            if let Some(argument) = call
                                .args
                                .get(i)
                                .filter(|argument| argument.spread.is_none())
                            {
                                self.validate_storage_source(&argument.expr, declared)?;
                            }
                        }
                        let promoted_typed_callback = direct_quickjs_typed_call
                            && matches!(
                                value,
                                HirExpr::FunctionRef(..)
                                    | HirExpr::Var(_)
                                    | HirExpr::Lambda(..)
                                    | HirExpr::TypedClosure(..)
                            )
                            && typed_callback_storage_return_matches(&actual, declared);
                        if promoted_typed_callback
                            && matches!(value, HirExpr::FunctionRef(..) | HirExpr::Var(_))
                        {
                            let (HirType::Function(params, ret)
                            | HirType::CallableFunction(params, _, _, ret)) = declared
                            else {
                                unreachable!()
                            };
                            self.validate_promise_callback_value(&value, params, Some(ret))
                                .map_err(|error| {
                                    format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                                })?;
                        }
                        let value = if live_method_json_argument || promoted_typed_callback {
                            Ok(value)
                        } else if class_object {
                            self.coerce_primitive_array_argument_aliasing(value, &HirType::Json)
                        } else if jit_structural_class_object {
                            self.coerce_jit_structural_class_argument(value, declared)
                        } else if direct_quickjs_typed_call && actual == HirType::Json {
                            Ok(value)
                        } else {
                            self.coerce_primitive_array_argument(value, declared)
                        }
                        .map_err(|error| {
                            format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                        })?;
                        if (direct_quickjs_typed_call || napi_constructor_typed_call)
                            && *declared == HirType::Json
                            && original_is_jsvalue
                            && Self::contains_live_host_jsvalue_json_conversion(&value)
                        {
                            Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    "__thaw_mark_typed_wrapper_json_argument".into(),
                                )),
                                vec![
                                    HirExpr::Lit(HirLit::Str(if direct_quickjs_typed_call {
                                        "direct-quickjs".into()
                                    } else {
                                        "napi-constructor".into()
                                    })),
                                    HirExpr::Lit(HirLit::Str(symbol.clone())),
                                    value,
                                ],
                            ))
                        } else {
                            Ok(value)
                        }
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                coerced_args.extend(args);
                let args = coerced_args;
                let params = if direct_quickjs_typed_call || quickjs_instance_typed_method {
                    params
                        .into_iter()
                        .enumerate()
                        .map(|(i, declared)| -> Result<_, String> {
                            let actual = args
                                .get(i)
                                .map(|value| self.infer_expr_type(value))
                                .transpose()?;
                            let live_method_json_argument = quickjs_instance_typed_method
                            && i > 0
                            && actual == Some(HirType::Json)
                                && call
                                    .args
                                    .get(i)
                                    .is_some_and(|argument| has_dynamic_storage(argument.expr.span()));
                            if live_method_json_argument {
                                Ok(HirType::Json)
                            } else if direct_quickjs_typed_call
                                && (actual == Some(HirType::Json)
                                    || actual.as_ref().is_some_and(|actual| {
                                    typed_callback_storage_return_matches(actual, &declared)
                                    }))
                            {
                                Ok(actual.expect("checked as Some"))
                            } else {
                                Ok(declared)
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    params
                };
                let mut result = HirExpr::DynamicCall(
                    DynamicSignature {
                        backend,
                        symbol,
                        params,
                        ret,
                    },
                    args,
                );
                if let Some((buffer, offset)) = read_sync_writeback {
                    let count = format!("__thaw_read_sync_count_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(count.clone(), HirType::F64);
                    let fill = HirExpr::DynamicCall(
                        DynamicSignature {
                            backend,
                            symbol: "node:fs::__thawTakeReadSync".into(),
                            params: Vec::new(),
                            ret: HirType::Bytes,
                        },
                        Vec::new(),
                    );
                    let copy = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_bytes_copy".into())),
                        vec![
                            fill,
                            buffer,
                            offset,
                            HirExpr::Lit(HirLit::F64(0.0)),
                            HirExpr::Lit(HirLit::F64(-1.0)),
                        ],
                    );
                    result = self.wrap_call_argument_bindings(
                        HirExpr::EvalThen(Box::new(copy), Box::new(HirExpr::Var(count.clone()))),
                        &[(count, HirType::F64, result)],
                    )?;
                }
                return self.wrap_call_argument_bindings(result, &argument_bindings);
            }
            // A plain (non-`__thaw_typed_*`) generic extern function's own
            // `params`/`ret` were collected using every type parameter
            // substituted with `Dynamic` (a placeholder so the signature
            // parses at all -- see `function_type_substitution`), not this
            // call's actual inferred types; re-derive them the same way
            // the `dynamic_symbol` branch above already does, or a
            // parameter/return that mentions a type parameter at all
            // (e.g. `nanoid<Type extends string>(size?: number): Type`'s
            // return) would stay `Dynamic` here regardless of `Type`
            // having been successfully inferred just above.
            let (params, ret) = if let Some(types) = &generic_types {
                let substitution = sig
                    .generic_type_params
                    .iter()
                    .cloned()
                    .zip(types.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let params = sig
                    .generic_param_patterns
                    .iter()
                    .zip(&sig.generic_param_optional)
                    .map(|(pattern, optional)| {
                        let ty = instantiate_generic_pattern(pattern, &substitution)?;
                        Ok(if *optional {
                            optional_parameter_type(ty)
                        } else {
                            ty
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let ret = match &sig.generic_return_type {
                    Some(declared) => resolve_ts_type_with_substitution(
                        declared,
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )?,
                    None => sig.ret.clone(),
                };
                (params, ret)
            } else {
                (sig.params.clone(), sig.ret.clone())
            };
            // See the identical comment on the `dynamic_symbol` branch
            // above: argument coercion against the real, substituted
            // parameter types (rather than the pre-substitution
            // `Dynamic` placeholder) was deferred to here, touching only
            // the arguments actually present up to `params.len()` -- a
            // variadic call's own trailing rest arguments (beyond
            // `params.len()`) pass through unchanged rather than being
            // truncated away, and an omitted optional trailing
            // parameter is left for arity checking elsewhere to handle,
            // not an error here.
            let fixed_count = params.len().min(args.len());
            let mut args = args.into_iter();
            let mut coerced_args = params
                .iter()
                .take(fixed_count)
                .enumerate()
                .map(|(i, declared)| {
                    let value = args.next().expect("fixed_count <= args.len()");
                    self.coerce_native_ffi_argument(value, declared)
                        .map_err(|error| {
                        format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            coerced_args.extend(args);
            let args = coerced_args;
            let param_count = params.len();
            let ffi_signature = FfiSignature {
                symbol: callee_name,
                params,
                variadic: sig.variadic,
                variadic_abi: crate::FfiVariadicAbi::Native,
                ret,
                error_abi: FfiErrorAbi::Direct,
                return_ownership: FfiOwnership::Borrowed,
                error_ownership: FfiOwnership::Borrowed,
                param_string_abis: vec![FfiStringAbi::NullTerminated; param_count],
                return_string_abi: FfiStringAbi::NullTerminated,
                calling_convention: FfiCallingConvention::C,
                aggregate_return_abi: FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            };
            let result = HirExpr::FfiCall(Box::new(ffi_signature), args);
            let result = self.adapt_native_ffi_result(result, call.span)?;
            return self.wrap_call_argument_bindings(result, &argument_bindings);
        }

        let lowered_name = self.specialize_generic_native_call(
            &callee_name,
            call,
            signature.as_ref(),
            generic_types.as_deref(),
            packed_native_rest_fixed_count,
            &mut args,
        )?;
        let result = if let Some(class_name) = class_method_guard {
            // The ordinary named-call path above has already selected its
            // overload, optional/default wrapper, rest ABI, and coercions.
            // Bind those final values once before checking identity: putting
            // the assertion in argument slot zero would reject a forged
            // receiver before later argument side effects could run.
            let mut final_bindings = Vec::with_capacity(args.len());
            let mut bound_args = Vec::with_capacity(args.len());
            for value in args {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_class_call_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                final_bindings.push((name.clone(), ty, value));
                bound_args.push(HirExpr::Var(name));
            }
            let receiver = bound_args
                .first_mut()
                .ok_or("class method call has no receiver")?;
            *receiver = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_assert_class_identity".into())),
                vec![
                    receiver.clone(),
                    HirExpr::Lit(HirLit::Str(class_name.to_string())),
                ],
            );
            let result = HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), bound_args);
            self.wrap_call_argument_bindings(result, &final_bindings)?
        } else {
            HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), args)
        };
        let physical_return = signature
            .as_ref()
            .map(|signature| signature.ret.clone())
            .or_else(|| local_function.as_ref().map(|(_, ret, _, _)| ret.clone()));
        let result = if let (Some(selected), Some(physical_return)) =
            (selected_public_return, physical_return.as_ref())
        {
            self.project_overload_call_result(result, physical_return, &selected)?
        } else {
            result
        };
        self.wrap_call_argument_bindings(result, &argument_bindings)
    }

    /// Lowers `obj[Symbol.x](...)` for the well-known symbols thaw models
    /// natively (`Symbol.iterator` and `Symbol.toPrimitive`).
    fn lower_well_known_symbol_call(
        &mut self,
        member: &MemberExpr,
        symbol: &str,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        match symbol {
            "iterator" => {
                if !call.args.is_empty() {
                    return Err("`[Symbol.iterator]()` expects no arguments".into());
                }
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                if let HirType::Union(members) = &receiver_type {
                    if members
                        .iter()
                        .all(|member| matches!(member, HirType::Array(_)))
                    {
                        let (receiver, array_type) =
                            self.lower_union_array_sequence(receiver, members)?;
                        let HirType::Array(element) = &array_type else {
                            unreachable!()
                        };
                        return self.lower_array_values(
                            receiver,
                            array_type.clone(),
                            element.as_ref().clone(),
                        );
                    }
                }
                // A class instance keeps `[Symbol.iterator]` as the method
                // `__thaw_symbol_iterator`: call it directly.
                if class_name_from_type(&receiver_type).is_some() {
                    let method = CallExpr {
                        span: call.span,
                        ctxt: call.ctxt,
                        callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                            span: call.span,
                            obj: member.obj.clone(),
                            prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                                "__thaw_symbol_iterator".into(),
                                call.span,
                            )),
                        }))),
                        args: Vec::new(),
                        type_args: None,
                    };
                    if let Ok(lowered) = self.lower_call(&method) {
                        return Ok(lowered);
                    }
                }
                // `obj[Symbol.iterator]()` reuses `Iterator.from(obj)`,
                // which already produces a native iterator for every
                // receiver thaw models.
                let from = CallExpr {
                    span: call.span,
                    ctxt: call.ctxt,
                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                        span: call.span,
                        obj: Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                            "Iterator".into(),
                            call.span,
                        ))),
                        prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                            "from".into(),
                            call.span,
                        )),
                    }))),
                    args: vec![swc_ecma_ast::ExprOrSpread {
                        spread: None,
                        expr: member.obj.clone(),
                    }],
                    type_args: None,
                };
                self.lower_expr(&Expr::Call(from))
            }
            "dispose" | "asyncDispose" => {
                // `obj[Symbol.dispose]()` / `obj[Symbol.asyncDispose]()`
                // invoke the method stored under the symbol's sentinel key.
                // Re-enter the ordinary field-call path with a computed
                // string key so the receiver-aware method call is reused.
                if !call.args.is_empty() {
                    return Err(format!("`[Symbol.{symbol}]()` expects no arguments"));
                }
                let synthetic = CallExpr {
                    span: call.span,
                    ctxt: call.ctxt,
                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                        span: call.span,
                        obj: member.obj.clone(),
                        prop: MemberProp::Computed(ComputedPropName {
                            span: call.span,
                            expr: Box::new(Expr::Lit(swc_ecma_ast::Lit::Str(swc_ecma_ast::Str {
                                span: call.span,
                                value: well_known_symbol_key(symbol).into(),
                                raw: None,
                            }))),
                        }),
                    }))),
                    args: Vec::new(),
                    type_args: None,
                };
                self.lower_call(&synthetic)
            }
            "toPrimitive" => {
                let receiver = self.lower_expr(&member.obj)?;
                let receiver_type = self.infer_expr_type(&receiver)?;
                if receiver_type != date_object_type() {
                    return Err(format!(
                        "`[Symbol.toPrimitive]` is only supported on a Date, got {receiver_type:?}"
                    ));
                }
                let hint = match call.args.as_slice() {
                    [] => None,
                    [argument] if argument.spread.is_none() => match argument.expr.as_ref() {
                        Expr::Lit(swc_ecma_ast::Lit::Str(value)) => {
                            Some(value.value.to_string_lossy().into_owned())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let timestamp = HirExpr::PropAccess(
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_date_assert_native_identity".into())),
                        vec![receiver],
                    )),
                    receiver_type,
                    "timestamp".to_string(),
                );
                match hint.as_deref() {
                    Some("number") => Ok(timestamp),
                    Some("string") | Some("default") => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_date_to_string".to_string())),
                        vec![timestamp],
                    )),
                    None => Err(
                        "`[Symbol.toPrimitive]` requires a literal \"number\"/\"string\"/\"default\" hint"
                            .into(),
                    ),
                    Some(other) => Err(format!("invalid `Symbol.toPrimitive` hint `{other}`")),
                }
            }
            other => Err(format!(
                "`[Symbol.{other}]()` is not supported (thaw models `Symbol.iterator` and `Symbol.toPrimitive`)"
            )),
        }
    }
}

fn typed_callback_storage_return_matches(actual: &HirType, declared: &HirType) -> bool {
    let (actual_params, actual_return) = match actual {
        HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
            (params, ret.as_ref())
        }
        _ => return false,
    };
    let (declared_params, declared_return) = match declared {
        HirType::Function(params, ret) | HirType::CallableFunction(params, _, _, ret) => {
            (params, ret.as_ref())
        }
        _ => return false,
    };
    fn promoted_return(physical: &HirType, source: &HirType) -> bool {
        match (physical, source) {
            (HirType::Json, HirType::Json | HirType::Dynamic) => false,
            (HirType::Json, _) => true,
            (HirType::Promise(physical), HirType::Promise(source)) => {
                promoted_return(physical, source)
            }
            _ => false,
        }
    }
    actual_params == declared_params && promoted_return(actual_return, declared_return)
}

// The ordinary Error classes still travel as a tagged string (`Error`
// annotations lower to `Str`; `__thaw_error_initialize_slot` has no codegen),
// so these helpers keep the pre-f15 representation instead of inventing a
// record layout the backend cannot build.
impl<'a> FnLowerer<'a> {
    /// `new Error(message?, options?)` and the other ordinary error classes.
    fn lower_builtin_error_object(
        &mut self,
        class_name: &str,
        args: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<HirExpr, String> {
        if args.iter().any(|argument| argument.spread.is_some()) {
            return Err(format!(
                "`new {class_name}()` does not support spread arguments"
            ));
        }
        if args.len() > 2 {
            return Err(format!(
                "`new {class_name}()` expects at most a message and options argument"
            ));
        }
        let message = match args.first() {
            Some(argument) => {
                let message = self.lower_expr(&argument.expr)?;
                self.coerce_primitive_to_string(message)?
            }
            None => HirExpr::Lit(HirLit::Str(String::new())),
        };
        let tagged = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_error_frame".to_string())),
            vec![HirExpr::Lit(HirLit::Str(class_name.to_string())), message],
        );
        let Some(options) = args.get(1) else {
            return Ok(tagged);
        };
        let options = self.lower_expr(&options.expr)?;
        let options_type = self.infer_expr_type(&options)?;
        self.lower_error_options(tagged, options, options_type)
    }

    /// Applies the `cause` of an Error constructor's options argument to the framed `tagged` error.
    /// A string cause rides in the frame (`\u{2}` suffix); anything else is stored hidden in the
    /// runtime side table (`__thaw_error_set_prop` with the `\u{2}cause` key), after a presence
    /// check, since `cause` is an own non-enumerable property only when the options have it.
    // ponytail: `{ cause: undefined }` is treated as no cause (Json cannot carry an own undefined).
    fn lower_error_options(
        &mut self,
        tagged: HirExpr,
        options: HirExpr,
        options_type: HirType,
    ) -> Result<HirExpr, String> {
        let key = HirExpr::Lit(HirLit::Str("cause".into()));
        let mut bindings = Vec::new();
        let (present, cause) = match &options_type {
            HirType::Object(fields) => {
                let Some((_, cause_type)) = fields.iter().find(|(name, _)| name == "cause") else {
                    return Ok(tagged);
                };
                let cause = HirExpr::PropAccess(
                    Box::new(options),
                    options_type.clone(),
                    "cause".to_string(),
                );
                if matches!(cause_type, HirType::Str) {
                    let with_marker = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![tagged, HirExpr::Lit(HirLit::Str("\u{2}".to_string()))],
                    );
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![with_marker, cause],
                    ));
                }
                (HirExpr::Lit(HirLit::Bool(true)), self.coerce_to_declared(&HirType::Json, cause)?)
            }
            HirType::Json | HirType::Dictionary(_) | HirType::JsValue | HirType::Dynamic => {
                let json_name = format!("__thaw_error_options_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(json_name.clone(), HirType::Json);
                let json = match options_type {
                    HirType::JsValue => HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_exception_json_from_handle".into())),
                        vec![options],
                    ),
                    HirType::Json => options,
                    _ => self.coerce_to_declared(&HirType::Json, options)?,
                };
                bindings.push((json_name.clone(), HirType::Json, json));
                let source = HirExpr::Var(json_name);
                let object_like = self.json_operand_is_object_like(source.clone(), &HirType::Json)?;
                let has = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_has".into())),
                    vec![source.clone(), key.clone()],
                );
                (
                    HirExpr::Conditional(
                        Box::new(object_like),
                        Box::new(has),
                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        HirType::Bool,
                    ),
                    HirExpr::JsonKey(Box::new(source), Box::new(key)),
                )
            }
            _ => return Ok(tagged),
        };
        let framed = format!("__thaw_error_cause_frame_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(framed.clone(), HirType::Str);
        let store = self.error_set_prop_call(&framed, "\u{2}cause", cause);
        bindings.insert(0, (framed.clone(), HirType::Str, tagged));
        let body = HirExpr::Block(vec![
            HirStmt::If(present, vec![HirStmt::Expr(store)], Vec::new()),
            HirStmt::Return(Some(HirExpr::Var(framed))),
        ]);
        self.wrap_call_argument_bindings(body, &bindings)
    }

    /// The family name an Error-family record inherits (`Error`, `TypeError`, ...), or `None` for
    /// a record that is not an Error subclass instance.
    fn error_record_default_name(fields: &[(Symbol, HirType)]) -> Option<String> {
        let chain = fields.first()?.0.strip_prefix("__thaw_class_identity_\u{1e}")?;
        chain.split('\u{1f}').find(|part| is_error_family_name(part)).map(str::to_string)
    }

    /// `Name: message` for a record, read from its own `name`/`message` fields (the header a stack
    /// starts with).
    fn object_stack_header(
        &mut self,
        object: &HirExpr,
        fields: &[(Symbol, HirType)],
    ) -> Result<HirExpr, String> {
        let text = |this: &mut Self, field: &str, default: &str| -> Result<HirExpr, String> {
            if fields.iter().any(|(name, ty)| {
                name == field && matches!(ty, HirType::Str | HirType::StrLiteral(_))
            }) {
                let read = this.lower_fixed_object_property_read(object.clone(), fields, field)?;
                this.coerce_primitive_to_string(read)
            } else {
                Ok(HirExpr::Lit(HirLit::Str(default.to_string())))
            }
        };
        let name = text(self, "name", "Error")?;
        let message = text(self, "message", "")?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_error_stack_with_frames".to_string())),
            vec![HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_frame".to_string())),
                vec![name, message],
            )],
        ))
    }

    /// `obj.stack` on a record without a `stack` slot. An Error subclass instance reads the stack
    /// remembered by `Error.captureStackTrace`/an assignment, else formats (and keeps) its
    /// `Name: message` header on first access. Any other record has no `stack` until one is
    /// stored, so it reads `undefined`.
    // ponytail: no call frames exist in a native binary, so a stack is only its header line.
    pub(crate) fn lower_object_stack_read(
        &mut self,
        object: HirExpr,
        fields: &[(Symbol, HirType)],
    ) -> Result<HirExpr, String> {
        let name = format!("__thaw_stack_owner_{}", self.next_binding);
        self.next_binding += 1;
        let object_type = HirType::Object(fields.to_vec());
        self.scope.insert(name.clone(), object_type.clone());
        let owner = HirExpr::Var(name.clone());
        let result = if Self::error_record_default_name(fields).is_some() {
            // V8 formats the header on first access: later `name`/`message` writes do not change it.
            let header = self.object_stack_header(&owner, fields)?;
            let freeze = HirStmt::If(
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_stack_has".into())),
                    vec![owner.clone()],
                ),
                Vec::new(),
                vec![HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_stack_set".into())),
                    vec![owner.clone(), header],
                ))],
            );
            HirExpr::Block(vec![
                freeze,
                HirStmt::Return(Some(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_stack_get".into())),
                    vec![owner, HirExpr::Lit(HirLit::Str(String::new()))],
                ))),
            ])
        } else {
            let captured = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_stack_get".into())),
                vec![owner.clone(), HirExpr::Lit(HirLit::Str(String::new()))],
            );
            HirExpr::Conditional(
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_stack_has".into())),
                    vec![owner],
                )),
                Box::new(self.coerce_to_declared(&HirType::Json, captured)?),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_json_undefined".into())),
                    Vec::new(),
                )),
                HirType::Json,
            )
        };
        self.wrap_call_argument_bindings(result, &[(name, object_type, object)])
    }

    /// `Error.captureStackTrace(record)`: store the current `Name: message` header as the record's
    /// `stack` (its slot when it declares one, else the side table).
    pub(crate) fn lower_object_capture_stack(
        &mut self,
        object: HirExpr,
        fields: &[(Symbol, HirType)],
    ) -> Result<HirExpr, String> {
        let name = format!("__thaw_capture_owner_{}", self.next_binding);
        self.next_binding += 1;
        let object_type = HirType::Object(fields.to_vec());
        self.scope.insert(name.clone(), object_type.clone());
        let owner = HirExpr::Var(name.clone());
        let header = self.object_stack_header(&owner, fields)?;
        let header = match fields.iter().find(|(field, _)| field == "stack") {
            Some((_, HirType::Str)) => Some(header),
            Some((_, HirType::Optional(inner))) if **inner == HirType::Str => {
                Some(HirExpr::OptionalSome(Box::new(header), HirType::Str))
            }
            Some(_) => None,
            None => {
                let text = format!("__thaw_capture_header_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(text.clone(), HirType::Str);
                let store = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_object_stack_set".into())),
                    vec![owner.clone(), HirExpr::Var(text.clone())],
                );
                return self.wrap_call_argument_bindings(
                    HirExpr::Block(vec![
                        HirStmt::Let(text, HirType::Str, header),
                        HirStmt::Expr(store),
                        HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                    ]),
                    &[(name, object_type, object)],
                );
            }
        };
        let mut statements = Vec::new();
        if let Some(value) = header {
            statements.push(HirStmt::Expr(HirExpr::PropAssign(
                Box::new(owner),
                object_type.clone(),
                "stack".to_string(),
                Box::new(value),
            )));
        }
        statements.push(HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))));
        self.wrap_call_argument_bindings(HirExpr::Block(statements), &[(name, object_type, object)])
    }

    /// A trusted Error-family record: the first field is the class-identity
    /// marker, kept hidden from enumeration like every other marker.
    // ponytail: `message_present`/`cause_present` (own-property presence) are
    // not representable without `__thaw_error_initialize_slot` codegen, so an
    // omitted message reads as "" and an omitted cause as undefined.
    fn lower_trusted_error_fields(
        &mut self,
        fields: Vec<(Symbol, HirExpr)>,
        _message_present: HirExpr,
        _cause_present: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let marker = match fields.first() {
            Some((name, _)) if name.starts_with("__thaw_class_identity_\u{1e}") => name.clone(),
            _ => return Err("Error record needs a leading class-identity marker".into()),
        };
        // A trusted record registers nominal identity (and hides the marker),
        // exactly what a constructor's ClassAlloc does for a class instance.
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_object_set_class_identity".to_string())),
            vec![
                HirExpr::ObjectLit(fields),
                HirExpr::Lit(HirLit::Str(marker)),
            ],
        ))
    }

    /// `AggregateError` copies its iterable into a new `errors` array. Keep
    /// typed arrays/tuples as-is; a promoted Json array is copied through the
    /// same host-aware slice used by array-rest lowering.
    fn native_aggregate_errors_as_json(
        &mut self,
        source: HirExpr,
        ty: &HirType,
    ) -> Result<HirExpr, String> {
        if matches!(ty, HirType::Array(_) | HirType::Tuple(_)) {
            return Ok(source);
        }
        if *ty == HirType::Json {
            let name = format!("__thaw_aggregate_errors_json_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), HirType::Json);
            let value = HirExpr::Var(name.clone());
            let is_array = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_is_array".into())),
                vec![value.clone()],
            );
            let invalid = HirExpr::Conditional(
                Box::new(is_array),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                HirType::Bool,
            );
            let error = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_error_frame".into())),
                vec![
                    HirExpr::Lit(HirLit::Str("TypeError".into())),
                    HirExpr::Lit(HirLit::Str(
                        "AggregateError errors must be an array".into(),
                    )),
                ],
            );
            let copy = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_json_array_slice".into())),
                vec![value, HirExpr::Lit(HirLit::F64(0.0))],
            );
            let body = HirExpr::Block(vec![
                HirStmt::If(invalid, vec![HirStmt::Throw(error)], Vec::new()),
                HirStmt::Return(Some(copy)),
            ]);
            Ok(HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    Vec::new(),
                    vec![HirParam {
                        name,
                        ty: HirType::Json,
                    }],
                    HirType::Json,
                    Box::new(body),
                )),
                vec![source],
            ))
        } else {
            Err("`AggregateError` errors must be a statically typed array or tuple".into())
        }
    }

    /// No live nominal projection of an options object exists yet; `None` tells
    /// the caller to use its typed/Json paths (it is not an empty `cause`).
    fn native_error_options_live_json(
        &mut self,
        _source: HirExpr,
        _ty: &HirType,
    ) -> Result<Option<HirExpr>, String> {
        Ok(None)
    }

    /// `(has cause, cause)` for a fixed-layout options object that has no
    /// `cause` slot and no accessor: the property is simply absent.
    fn native_error_options_cause(
        &mut self,
        _source: HirExpr,
        ty: &HirType,
    ) -> Result<Option<(HirExpr, HirExpr)>, String> {
        Ok(matches!(ty, HirType::Object(_)).then(|| {
            (
                HirExpr::Lit(HirLit::Bool(false)),
                HirExpr::Lit(HirLit::Undefined),
            )
        }))
    }
}

impl<'a> FnLowerer<'a> {
    /// A Json-typed `.includes` receiver can either be an ordinary decoded
    /// value or a live QuickJS handle carried through the JSON bridge. Reuse
    /// the selected-method path in both cases: preserve an existing handle
    /// when present, otherwise materialize the captured Json value as a live
    /// value before normal JavaScript property lookup and invocation.
    fn lower_json_includes_method_call(
        &mut self,
        member: &MemberExpr,
        args: &[swc_ecma_ast::ExprOrSpread],
        expected: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let receiver = self.lower_expr(&member.obj)?;
        let receiver_type = self.infer_expr_type(&receiver)?;
        if receiver_type != HirType::Json {
            return Err(format!(
                "dynamic `.includes()` expected Json receiver, got {receiver_type:?}"
            ));
        }
        let json_name = format!("__thaw_dynamic_includes_json_{}", self.next_binding);
        self.next_binding += 1;
        let handle_name = format!("__thaw_dynamic_includes_handle_{}", self.next_binding);
        self.next_binding += 1;
        let live_name = format!("__thaw_dynamic_includes_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(json_name.clone(), HirType::Json);
        self.scope.insert(handle_name.clone(), HirType::I64);
        self.scope.insert(live_name.clone(), HirType::JsValue);
        let json = HirExpr::Var(json_name.clone());
        let handle = HirExpr::Var(handle_name.clone());
        let borrowed_handle = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_borrowed_handle_id".into())),
            vec![json.clone()],
        );
        let live = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(handle.clone()),
                Box::new(HirExpr::Lit(HirLit::I64(0))),
            )),
            Box::new(HirExpr::Call(
                Box::new(HirExpr::Var("retainDynamicJson".into())),
                vec![json.clone()],
            )),
            Box::new(HirExpr::JsonAsNative(Box::new(json), HirType::JsValue)),
            HirType::JsValue,
        );
        let receiver_ident =
            swc_ecma_ast::Ident::new_no_ctxt(live_name.clone().into(), member.span);
        let result = self.lower_dynamic_value_method_call(
            &Expr::Ident(receiver_ident),
            "includes",
            args,
            expected,
        )?;
        self.wrap_call_argument_bindings(
            result,
            &[
                (json_name, HirType::Json, receiver),
                (handle_name, HirType::I64, borrowed_handle),
                (live_name, HirType::JsValue, live),
            ],
        )
    }
}

/// `((t: any) => BigInt(Number(t[0])) * BigInt(1000000000) + BigInt(Number(t[1])))(process.hrtime())`
fn hrtime_bigint_expression(process: &str) -> Expr {
    use swc_common::{SyntaxContext, DUMMY_SP};
    use swc_ecma_ast::*;
    let ident = |name: &str| Ident::new_no_ctxt(name.into(), DUMMY_SP);
    let call = |callee: Expr, args: Vec<Expr>| {
        Expr::Call(CallExpr {
            span: DUMMY_SP,
            ctxt: SyntaxContext::empty(),
            callee: Callee::Expr(Box::new(callee)),
            args: args.into_iter().map(|expr| ExprOrSpread { spread: None, expr: Box::new(expr) }).collect(),
            type_args: None,
        })
    };
    let number = |value: f64| Expr::Lit(Lit::Num(swc_ecma_ast::Number { span: DUMMY_SP, value, raw: None }));
    let part = |index: f64| {
        let element = Expr::Member(MemberExpr {
            span: DUMMY_SP,
            obj: Box::new(Expr::Ident(ident("__thaw_hr"))),
            prop: MemberProp::Computed(ComputedPropName { span: DUMMY_SP, expr: Box::new(number(index)) }),
        });
        call(Expr::Ident(ident("BigInt")), vec![call(Expr::Ident(ident("Number")), vec![element])])
    };
    let scale = call(Expr::Ident(ident("BigInt")), vec![number(1_000_000_000.0)]);
    let binary = |op: BinaryOp, left: Expr, right: Expr| {
        Expr::Bin(BinExpr { span: DUMMY_SP, op, left: Box::new(left), right: Box::new(right) })
    };
    let body = binary(BinaryOp::Add, binary(BinaryOp::Mul, part(0.0), scale), part(1.0));
    let parameter = Pat::Ident(BindingIdent {
        id: ident("__thaw_hr"),
        type_ann: Some(Box::new(TsTypeAnn {
            span: DUMMY_SP,
            type_ann: Box::new(TsType::TsKeywordType(TsKeywordType { span: DUMMY_SP, kind: TsKeywordTypeKind::TsAnyKeyword })),
        })),
    });
    let arrow = Expr::Arrow(ArrowExpr {
        span: DUMMY_SP,
        ctxt: SyntaxContext::empty(),
        params: vec![parameter],
        body: Box::new(ArrowFunctionBody::Expr(Box::new(body))),
        is_async: false,
        is_generator: false,
        type_params: None,
        return_type: None,
    });
    let hrtime = Expr::Member(MemberExpr {
        span: DUMMY_SP,
        obj: Box::new(Expr::Ident(ident(process))),
        prop: MemberProp::Ident(IdentName::new("hrtime".into(), DUMMY_SP)),
    });
    call(Expr::Paren(ParenExpr { span: DUMMY_SP, expr: Box::new(arrow) }), vec![call(hrtime, Vec::new())])
}
