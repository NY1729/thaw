impl<'ctx> HirCompiler<'ctx> {
    fn llvm_symbol_for(name: &str) -> String {
        if name == "main" {
            USER_MAIN_SYMBOL.to_string()
        } else {
            name.to_string()
        }
    }

    fn basic_type(&self, ty: &HirType) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            HirType::F64 => Ok(self.context.f64_type().into()),
            HirType::I64 => Ok(self.context.i64_type().into()),
            HirType::Bool => Ok(self.context.bool_type().into()),
            // `void` has no top-level return value, but tagged containers
            // such as `void | T` still need a physical placeholder slot.
            HirType::Void => Ok(self.context.i32_type().into()),
            HirType::Undefined => Ok(self.context.bool_type().into()),
            // Strings and arrays are both represented as a single opaque
            // pointer at the LLVM level; what they point to differs (a
            // C string vs. a [len][elements...] buffer).
            HirType::Str => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            HirType::Array(elem) => {
                self.basic_type(elem)?;
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // `Map`/`Set` are both a single opaque pointer to an
            // arena-allocated hash-table header (see `thaw-runtime`'s
            // `thaw_map_new`); `basic_type` on the key/value types just
            // validates they're representable at all, the same way it
            // does for array elements.
            HirType::Map(key, value) => {
                self.basic_type(key)?;
                self.basic_type(value)?;
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::Set(element) => {
                self.basic_type(element)?;
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::Tuple(elements) => {
                for element in elements {
                    self.basic_type(element)?;
                }
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // Objects are represented the same way: a single opaque pointer
            // to an arena-allocated buffer of contiguous `f64` fields (see
            // `compile_object_lit`). Unlike arrays there's no length
            // header -- field count/order is static, part of the type.
            // Every field is one word (8 bytes on this target) regardless
            // of its own type: `f64`/`bool`/pointer values (Str/Array/
            // Object/Json) are all word-sized, so a field can be any type
            // `basic_type` itself accepts -- including another `Object`,
            // recursively. This call validates each field is representable
            // at all (erroring on e.g. `Promise`); it doesn't need to
            // guard against cycles itself since a self-referential
            // interface is already rejected at lowering time
            // (`thaw_hir::lower::resolve_interface`), so a genuinely
            // cyclic `HirType::Object` should never reach codegen.
            HirType::Object(fields) => {
                for (name, ty) in fields {
                    self.basic_type(ty)
                        .map_err(|e| format!("object field `{name}`: {e}"))?;
                }
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // A `Json` value is an opaque pointer to a boxed, dynamically-
            // typed `serde_json::Value` (thaw-std), same representation
            // family as everything else -- only `thaw_json_*` (thaw-std)
            // ever dereferences it.
            HirType::Json | HirType::Dictionary(_) => {
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::JsValue => Ok(self.context.i64_type().into()),
            HirType::Null => Ok(self.context.bool_type().into()),
            HirType::Function(_, _) | HirType::CallableFunction(..) => {
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::Promise(_) => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            HirType::Optional(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.bool_type().into(), payload], false)
                    .into())
            }
            HirType::Nullable(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.bool_type().into(), payload], false)
                    .into())
            }
            HirType::Nullish(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.i8_type().into(), payload], false)
                    .into())
            }
            HirType::Union(elements) => {
                if elements.is_empty() {
                    return Err("empty union type is not supported".into());
                }
                for element in elements {
                    self.basic_type(element)?;
                }
                Ok(self
                    .context
                    .struct_type(
                        &[
                            self.context.i8_type().into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                    .into())
            }
            other => Err(format!(
                "Phase 1/2 codegen does not support type {other:?} yet"
            )),
        }
    }

    fn function_type(
        &self,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<FunctionType<'ctx>, String> {
        let mut lowered = vec![BasicMetadataTypeEnum::from(
            self.context.ptr_type(AddressSpace::default()),
        )];
        lowered.extend(
            params
                .iter()
                .map(|ty| self.basic_type(ty).map(BasicMetadataTypeEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if *ret == HirType::Void {
            Ok(self.context.void_type().fn_type(&lowered, false))
        } else {
            Ok(self.basic_type(ret)?.fn_type(&lowered, false))
        }
    }

    fn compile_ignored_this_adapter(
        &mut self,
        target: FunctionValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        name: &str,
    ) -> Result<FunctionValue<'ctx>, String> {
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let adapter = self.module.add_function(
            name,
            self.function_type(&this_params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self
            .builder
            .get_insert_block()
            .ok_or("this-aware adapter must be emitted inside a function")?;
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let mut arguments = vec![BasicMetadataValueEnum::from(
            adapter.get_nth_param(0).unwrap(),
        )];
        arguments.extend(
            adapter
                .get_param_iter()
                .skip(2)
                .map(BasicMetadataValueEnum::from),
        );
        let call = self
            .builder
            .build_call(target, &arguments, "invoke_ignoring_this")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("this-aware adapter target returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(parent);
        Ok(adapter)
    }

    fn allocate_variable_cell(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let function = self.current_function();
        let entry = function
            .get_first_basic_block()
            .ok_or("variable allocation requires a function entry")?;
        let builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            builder.position_before(&first);
        } else {
            builder.position_at_end(entry);
        }
        builder
            .build_alloca(ty, &format!("{name}_cell"))
            .map_err(|error| error.to_string())
    }

    fn allocate_arena_cell(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let size = ty
            .size_of()
            .ok_or_else(|| format!("variable `{name}` has an unsized LLVM type"))?;
        let cell = self
            .builder
            .build_call(
                alloc,
                &[size.into(), i64_type.const_int(8, false).into()],
                &format!("{name}_cell"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a variable cell")?
            .into_pointer_value();
        Ok(cell)
    }

    fn declare_function(&mut self, func: &HirFunction) -> Result<FunctionValue<'ctx>, String> {
        let param_types = func
            .params
            .iter()
            .map(|p| self.basic_type(&p.ty).map(BasicMetadataTypeEnum::from))
            .collect::<Result<Vec<_>, _>>()?;

        let frame_split = self.frame_await_plan(func)?.is_some();
        let fn_type = if frame_split {
            self.context
                .ptr_type(AddressSpace::default())
                .fn_type(&param_types, false)
        } else {
            match &func.ret {
                HirType::Void => self.context.void_type().fn_type(&param_types, false),
                ret => self.basic_type(ret)?.fn_type(&param_types, false),
            }
        };

        let symbol = Self::llvm_symbol_for(&func.name);
        let linkage = if func.name == "__thaw_artifact_metadata" {
            Linkage::External
        } else {
            Linkage::Internal
        };
        Ok(self.module.add_function(&symbol, fn_type, Some(linkage)))
    }

    /// Declares an ambient (`declare function`) signature as an `extern
    /// "C"` symbol -- see docs/design/bridge.md sections 5/6. The final
    /// link step must resolve it from somewhere else (a real native
    /// library today; thaw-registry eventually).
    ///
    /// The declared LLVM parameter list is *not* simply one slot per HIR
    /// parameter: `Array`/`Object` params are expanded into the real C ABI
    /// shape a native library actually expects (see `ffi_param_types`),
    /// matching the argument list `compile_ffi_call` builds at each call
    /// site. Aggregate returns use an explicit, portable C shape and are
    /// copied into Thaw's arena representation after the call:
    /// `number[]` is `{ const double *data; int64_t len; }`, while an object
    /// is a value struct whose fields remain in declaration order.
    fn declare_extern_function(
        &mut self,
        sig: &FfiSignature,
    ) -> Result<FunctionValue<'ctx>, String> {
        let supports_owned_return =
            Self::supports_owned_ffi_return(&sig.ret, sig.aggregate_return_abi);
        if sig.return_ownership != FfiOwnership::Borrowed && !supports_owned_return {
            return Err(format!(
                "FFI function `{}` uses non-borrowed return ownership, which is currently supported only for string, primitive arrays, and portable/packed object returns containing supported string or primitive-array leaves",
                sig.symbol
            ));
        }
        if sig.error_abi == FfiErrorAbi::Direct && sig.error_ownership != FfiOwnership::Borrowed {
            return Err(format!(
                "FFI function `{}` cannot configure error ownership with the direct error ABI",
                sig.symbol
            ));
        }
        for ownership in [&sig.return_ownership, &sig.error_ownership] {
            let destroy = match ownership {
                FfiOwnership::Owned { destroy } => Some(destroy),
                FfiOwnership::ArenaCopy {
                    destroy: Some(destroy),
                } => Some(destroy),
                _ => None,
            };
            if let Some(destroy) = destroy {
                if self.module.get_function(destroy).is_none() {
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let fn_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);
                    self.module
                        .add_function(destroy, fn_ty, Some(Linkage::External));
                }
            }
        }
        let param_types = self.ffi_param_types(&sig.params, &sig.param_string_abis)?;

        let return_type = self.ffi_call_return_type(sig)?;
        let fn_type = if Self::uses_indirect_ffi_return(sig) {
            let mut indirect_params = Vec::with_capacity(param_types.len() + 1);
            indirect_params.push(
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
                    .into(),
            );
            indirect_params.extend(param_types);
            self.context
                .void_type()
                .fn_type(&indirect_params, sig.variadic.is_some())
        } else if let Some(return_type) = return_type {
            return_type.fn_type(&param_types, sig.variadic.is_some())
        } else {
            self.context
                .void_type()
                .fn_type(&param_types, sig.variadic.is_some())
        };

        let function = self
            .module
            .add_function(&sig.symbol, fn_type, Some(Linkage::External));
        function.set_call_conventions(Self::ffi_calling_convention(sig.calling_convention));
        Ok(function)
    }
}
