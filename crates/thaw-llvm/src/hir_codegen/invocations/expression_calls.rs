impl<'ctx> HirCompiler<'ctx> {
    fn compile_expression_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // A zero-argument identity lambda over its sole JsValue capture is
        // exactly the captured value. Inline this pure case before closure
        // construction so it does not promote/retain a capture environment.
        if let HirExpr::Lambda(captures, params, ret, body) = callee {
            if captures.len() == 1
                && captures[0].ty == HirType::JsValue
                && params.is_empty()
                && *ret == HirType::JsValue
                && args.is_empty()
                && matches!(body.as_ref(), HirExpr::Var(name) if name == &captures[0].name)
            {
                return self.compile_expr(body);
            }
        }
        if let HirExpr::Lambda(_, params, ret, body) = callee {
            if thaw_hir::is_inlinable_iife(callee, args) {
                let plan = thaw_hir::iife_body_plan(body).expect("checked by is_inlinable_iife");
                return self.compile_inline_lambda_call(params, ret, &plan, args);
            }
        }
        if let HirExpr::Lambda(_, params, ret, _) = callee {
            let parameter_types = params
                .iter()
                .map(|parameter| parameter.ty.clone())
                .collect::<Vec<_>>();
            return self.compile_closure_call(
                callee,
                &parameter_types,
                ret,
                args,
                "inline closure",
            );
        }
        if let HirExpr::PropAccess(_, HirType::Object(fields), field) = callee {
            if let Some((_, HirType::Function(params, ret))) =
                fields.iter().find(|(name, _)| name == field)
            {
                return self.compile_closure_call(
                    callee,
                    params,
                    ret,
                    args,
                    &format!("method `{field}`"),
                );
            }
        }
        if let Some(HirType::Function(params, ret)) = self.expr_hir_type(callee) {
            return self.compile_closure_call(
                callee,
                &params,
                ret.as_ref(),
                args,
                "function expression",
            );
        }
        if let Some(HirType::CallableFunction(mut params, _, rest, ret)) =
            self.expr_hir_type(callee)
        {
            if let Some(rest) = rest {
                params.push(HirType::Array(rest));
            }
            return self.compile_closure_call(
                callee,
                &params,
                ret.as_ref(),
                args,
                "callable function expression",
            );
        }
        Err("call target is not a compiled function value".to_string())
    }

    /// `(param) => body` called at once (see `thaw_hir::is_inlinable_iife`): the parameters are
    /// fresh locals and the body is compiled here, with no closure allocation or indirect call.
    fn compile_inline_lambda_call(
        &mut self,
        params: &[HirParam],
        ret: &HirType,
        plan: &thaw_hir::IifePlan,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let values = args.iter().map(|arg| self.compile_expr(arg)).collect::<Result<Vec<_>, _>>()?;
        let mut shadowed = Vec::with_capacity(params.len());
        for (param, value) in params.iter().zip(values) {
            let ty = self.basic_type(&param.ty)?;
            let slot = self.allocate_variable_cell(ty, &param.name)?;
            self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
            shadowed.push((
                param.name.clone(),
                self.variables.insert(param.name.clone(), (slot, ty)),
                self.variable_hir_types.insert(param.name.clone(), param.ty.clone()),
            ));
        }
        let result = self.compile_inline_lambda_body(ret, plan);
        for (name, variable, hir_type) in shadowed.into_iter().rev() {
            match variable {
                Some(variable) => self.variables.insert(name.clone(), variable),
                None => self.variables.remove(&name),
            };
            match hir_type {
                Some(hir_type) => self.variable_hir_types.insert(name, hir_type),
                None => self.variable_hir_types.remove(&name),
            };
        }
        result
    }

    /// The body of an in-place lambda: each early-exit guard branches to its `return`/`throw`, the
    /// final value falls through. A `return` stores into one result slot read after the join.
    fn compile_inline_lambda_body(
        &mut self,
        ret: &HirType,
        plan: &thaw_hir::IifePlan,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if plan.guards.is_empty() {
            return self.compile_expr(&plan.value);
        }
        let ty = self.basic_type(ret)?;
        let slot = self.allocate_variable_cell(ty, "iife_result")?;
        let function = self.current_function();
        let done = self.context.append_basic_block(function, "iife_done");
        for (test, exit) in &plan.guards {
            let test = self.compile_expr(test)?.into_int_value();
            let exit_block = self.context.append_basic_block(function, "iife_exit");
            let next = self.context.append_basic_block(function, "iife_next");
            self.builder
                .build_conditional_branch(test, exit_block, next)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(exit_block);
            match exit {
                thaw_hir::IifeExit::Return(value) => {
                    let value = self.compile_expr(value)?;
                    self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
                    self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                }
                thaw_hir::IifeExit::Throw(thrown) => {
                    if !self.compile_block(&[HirStmt::Throw(thrown.clone())])? {
                        return Err("an in-place lambda's throw guard must not fall through".into());
                    }
                }
            }
            self.builder.position_at_end(next);
        }
        let value = self.compile_expr(&plan.value)?;
        self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.builder.build_load(ty, slot, "iife_value").map_err(|error| error.to_string())
    }
}
