/// The engine behind live `JsValue` handles in this build, chosen once from the routing. Each
/// handle intrinsic maps to one method here, so a thaw-js build never names a QuickJS symbol;
/// add the remaining intrinsics (get_property, call_selected_method_graph, ...) the same way.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JsEngine {
    QuickJs,
    ThawJs,
}

impl<'ctx> HirCompiler<'ctx> {
    fn js_engine(&self) -> JsEngine {
        if self.thaw_js_routed {
            JsEngine::ThawJs
        } else {
            JsEngine::QuickJs
        }
    }

    /// Record that the program holds live JS handles (QuickJS: starts the runtime; thaw-js: loads the archive).
    fn js_mark_handles_used(&mut self) {
        match self.js_engine() {
            JsEngine::QuickJs => {
                self.uses_quickjs = true;
                self.uses_quickjs_handles = true;
            }
            JsEngine::ThawJs => self.uses_thaw_js = true,
        }
    }

    /// Retain the value graph wire `wire` as a handle: `{ handle: i64, error: ptr }`.
    fn js_retain_graph(
        &mut self,
        wire: BasicValueEnum<'ctx>,
        name: &str,
    ) -> Result<inkwell::values::StructValue<'ctx>, String> {
        let symbol = match self.js_engine() {
            JsEngine::QuickJs => "thaw_js_retain_graph_result",
            JsEngine::ThawJs => "thaw_tjs_retain_graph_result",
        };
        Ok(self
            .builder
            .build_call(self.module.get_function(symbol).unwrap(), &[wire.into()], name)
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("graph retain returned no result")?
            .into_struct_value())
    }
}
