// Native timers: `setTimeout`/`setInterval`/`setImmediate` with a zero-argument native callback lower
// to `__thaw_timer_*` builtins (thaw-runtime's timer queue) instead of the QuickJS globals, so a
// timer-only program never creates a JS context. Ineligible calls return `Ok(None)` and take the
// ordinary path; the CLI re-lowers without this mode when the program still needs QuickJS.
impl<'a> FnLowerer<'a> {
    fn timer_global_unshadowed(&self, name: &str) -> bool {
        !self.scope.contains_key(name) && !self.signatures.contains_key(name)
    }

    /// `kind` (0 timeout, 1 interval, 2 immediate) when `expr` is an eligible `set*(callback[, delay])` call.
    fn native_timer_set_kind(&self, expr: &Expr) -> Option<u8> {
        let Expr::Call(call) = expr else { return None };
        let Callee::Expr(callee) = &call.callee else { return None };
        let Expr::Ident(name) = callee.as_ref() else { return None };
        let (kind, max_args) = match name.sym.as_ref() {
            "setTimeout" => (0, 2),
            "setInterval" => (1, 2),
            "setImmediate" => (2, 1),
            _ => return None,
        };
        if !self.timer_global_unshadowed(name.sym.as_ref())
            || call.type_args.is_some()
            || call.args.is_empty()
            || call.args.len() > max_args
            || call.args.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        match call.args[0].expr.as_ref() {
            Expr::Arrow(arrow) if arrow.params.is_empty() && !arrow.is_async && !arrow.is_generator => Some(kind),
            Expr::Fn(function) if function.function.params.is_empty()
                && !function.function.is_async
                && !function.function.is_generator => Some(kind),
            Expr::Ident(_) => Some(kind),
            _ => None,
        }
    }

    fn is_native_timer_handle(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Ident(ident) => self.scope.get(&self.resolve_binding(ident.sym.as_ref())) == Some(&HirType::I64),
            other => self.native_timer_set_kind(other).is_some(),
        }
    }

    fn try_lower_native_timer(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        if !native_timers() || call.type_args.is_some() {
            return Ok(None);
        }
        let Callee::Expr(callee) = &call.callee else { return Ok(None) };
        match callee.as_ref() {
            Expr::Ident(name) => {
                if self.native_timer_set_kind(&Expr::Call(call.clone())).is_some() {
                    return self.lower_native_timer_set(call).map(Some);
                }
                if matches!(name.sym.as_ref(), "clearTimeout" | "clearInterval" | "clearImmediate")
                    && self.timer_global_unshadowed(name.sym.as_ref())
                    && call.args.len() == 1
                    && call.args[0].spread.is_none()
                    && self.is_native_timer_handle(&call.args[0].expr)
                {
                    let handle = self.lower_expr(&call.args[0].expr)?;
                    return Ok(Some(HirExpr::Call(Box::new(HirExpr::Var("__thaw_timer_clear".into())), vec![handle])));
                }
                Ok(None)
            }
            Expr::Member(member) => {
                let MemberProp::Ident(prop) = &member.prop else { return Ok(None) };
                let builtin = match prop.sym.as_ref() {
                    "ref" => "__thaw_timer_ref",
                    "unref" => "__thaw_timer_unref",
                    "refresh" => "__thaw_timer_refresh",
                    "hasRef" => "__thaw_timer_has_ref",
                    _ => return Ok(None),
                };
                if !call.args.is_empty() || !self.is_native_timer_handle(&member.obj) {
                    return Ok(None);
                }
                let handle = self.lower_expr(&member.obj)?;
                Ok(Some(HirExpr::Call(Box::new(HirExpr::Var(builtin.into())), vec![handle])))
            }
            _ => Ok(None),
        }
    }

    fn lower_native_timer_set(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        let kind = self
            .native_timer_set_kind(&Expr::Call(call.clone()))
            .ok_or("native timer call is not eligible")?;
        let callback = match call.args[0].expr.as_ref() {
            Expr::Arrow(arrow) => self.lower_contextual_arrow(arrow, &[], None)?,
            other => self.lower_expr(other)?,
        };
        match self.infer_expr_type(&callback)? {
            HirType::Function(params, ret)
                if params.is_empty()
                    && matches!(*ret, HirType::Void | HirType::F64 | HirType::Bool | HirType::Undefined) => {}
            other => return Err(format!("native timer callback has type {other:?}")),
        }
        let delay = match call.args.get(1) {
            Some(argument) => {
                let delay = self.lower_expr_with_expected_type(&argument.expr, Some(&HirType::F64))?;
                if self.infer_expr_type(&delay)? != HirType::F64 {
                    return Err("native timer delay must be a number".into());
                }
                delay
            }
            None => HirExpr::Lit(HirLit::F64(0.0)),
        };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_timer_set".into())),
            vec![HirExpr::Lit(HirLit::F64(f64::from(kind))), delay, callback],
        ))
    }
}
