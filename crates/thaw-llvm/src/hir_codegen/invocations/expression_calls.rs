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
}
