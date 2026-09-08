impl<'ctx> HirCompiler<'ctx> {
    fn compile_variable_call(
        &mut self,
        name: &str,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        match self.variable_hir_types.get(name).cloned()? {
            HirType::Function(params, ret) => {
                Some(self.compile_closure_call(callee, &params, &ret, args, name))
            }
            HirType::CallableFunction(mut params, _, rest, ret) => {
                if let Some(rest) = rest {
                    params.push(HirType::Array(rest));
                }
                Some(self.compile_closure_call(callee, &params, &ret, args, name))
            }
            _ => None,
        }
    }
}
