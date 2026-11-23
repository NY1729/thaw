impl<'ctx> HirCompiler<'ctx> {
    /// Record that the program holds live JS handles: the thaw-js archive must be linked.
    fn js_mark_handles_used(&mut self) {
        self.uses_thaw_js = true;
        self.uses_thaw_js_handles = true;
    }

    /// A `thaw_js_*` runtime symbol still called after codegen has no thaw-js definition (every
    /// ported symbol is declared under its `thaw_tjs_*` name), so the program cannot link: report
    /// every such symbol instead of failing in the linker.
    pub(crate) fn reject_unported_js_symbols(&self) -> Result<(), String> {
        let mut called: Vec<String> = self
            .module
            .get_functions()
            .filter(|function| {
                function.get_name().to_string_lossy().starts_with("thaw_js_")
                    && function.as_global_value().as_pointer_value().get_first_use().is_some()
            })
            .map(|function| function.get_name().to_string_lossy().into_owned())
            .collect();
        if called.is_empty() {
            return Ok(());
        }
        called.sort();
        Err(format!(
            "no thaw-js implementation of the runtime symbol(s) {}",
            called.join(", ")
        ))
    }

    /// Whether the program holds live JS handles.
    fn js_handles_used(&self) -> bool {
        self.uses_thaw_js_handles
    }

    /// Retain the value graph wire `wire` as a handle: `{ handle: i64, error: ptr }`.
    fn js_retain_graph(
        &mut self,
        wire: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<inkwell::values::StructValue<'ctx>, String> {
        Ok(self
            .builder
            .build_call(
                self.module.get_function("thaw_tjs_retain_graph_result").unwrap(),
                &[wire.into()],
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("graph retain returned no result")?
            .into_struct_value())
    }
}
