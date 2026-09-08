impl<'ctx> HirCompiler<'ctx> {
    fn compile_expression_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
