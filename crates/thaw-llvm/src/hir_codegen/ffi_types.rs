impl<'ctx> HirCompiler<'ctx> {
    fn supports_owned_ffi_return(ty: &HirType, aggregate_abi: FfiAggregateAbi) -> bool {
        match ty {
            HirType::Str => true,
            HirType::Array(element) => **element == HirType::F64,
            HirType::Object(fields) if aggregate_abi != FfiAggregateAbi::Internal => {
                fields
                    .iter()
                    .all(|(_, field)| Self::supports_ffi_aggregate_field(field))
                    && fields
                        .iter()
                        .any(|(_, field)| Self::ffi_aggregate_field_has_owned_leaf(field))
            }
            _ => false,
        }
    }

    fn supports_ffi_aggregate_field(ty: &HirType) -> bool {
        match ty {
            HirType::F64 | HirType::I64 | HirType::Bool | HirType::Str => true,
            HirType::Array(element) => **element == HirType::F64,
            HirType::Object(fields) => fields
                .iter()
                .all(|(_, field)| Self::supports_ffi_aggregate_field(field)),
            _ => false,
        }
    }

    fn ffi_aggregate_field_has_owned_leaf(ty: &HirType) -> bool {
        match ty {
            HirType::Str => true,
            HirType::Array(element) => **element == HirType::F64,
            HirType::Object(fields) => fields
                .iter()
                .any(|(_, field)| Self::ffi_aggregate_field_has_owned_leaf(field)),
            _ => false,
        }
    }

    fn ffi_calling_convention(convention: FfiCallingConvention) -> u32 {
        match convention {
            FfiCallingConvention::C => 0,
            FfiCallingConvention::Fast => 8,
            FfiCallingConvention::Cold => 9,
        }
    }

    fn uses_indirect_ffi_return(sig: &FfiSignature) -> bool {
        if !matches!(sig.ret, HirType::Array(_) | HirType::Object(_)) {
            return false;
        }
        if let Some(layout) = &sig.aggregate_return_layout {
            return layout.indirect;
        }
        sig.aggregate_return_abi == FfiAggregateAbi::Packed
            || Self::ffi_result_storage_bytes(sig) > 16
    }

    fn ffi_result_storage_bytes(sig: &FfiSignature) -> u64 {
        let (value_size, value_align) = sig
            .aggregate_return_layout
            .as_ref()
            .map(|layout| (layout.size, u64::from(layout.alignment)))
            .unwrap_or_else(|| {
                Self::ffi_aggregate_storage_layout(&sig.ret, sig.aggregate_return_abi)
            });
        if sig.error_abi == FfiErrorAbi::Direct {
            value_size
        } else {
            Self::align_to(value_size, 8) + 8_u64.max(value_align)
        }
    }

    fn ffi_aggregate_storage_layout(ty: &HirType, aggregate_abi: FfiAggregateAbi) -> (u64, u64) {
        match ty {
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue
                )
                    && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                (
                    16,
                    if aggregate_abi == FfiAggregateAbi::Packed {
                        1
                    } else {
                        8
                    },
                )
            }
            HirType::Object(fields) if aggregate_abi != FfiAggregateAbi::Internal => {
                if aggregate_abi == FfiAggregateAbi::Packed {
                    let size = fields
                        .iter()
                        .map(|(_, ty)| Self::ffi_object_field_layout(ty, aggregate_abi).0)
                        .sum();
                    (size, 1)
                } else {
                    let mut size = 0;
                    let mut alignment = 1;
                    for (_, ty) in fields {
                        let (field_size, field_alignment) =
                            Self::ffi_object_field_layout(ty, aggregate_abi);
                        size = Self::align_to(size, field_alignment) + field_size;
                        alignment = alignment.max(field_alignment);
                    }
                    (Self::align_to(size, alignment), alignment)
                }
            }
            _ => (8, 8),
        }
    }

    fn ffi_object_field_layout(ty: &HirType, aggregate_abi: FfiAggregateAbi) -> (u64, u64) {
        match ty {
            HirType::Bool => (1, 1),
            HirType::F64 | HirType::I64 => (8, 8),
            HirType::Array(_) | HirType::Object(_)
                if aggregate_abi != FfiAggregateAbi::Internal =>
            {
                Self::ffi_aggregate_storage_layout(ty, aggregate_abi)
            }
            _ => (8, 8),
        }
    }

    fn align_to(offset: u64, alignment: u64) -> u64 {
        offset.div_ceil(alignment) * alignment
    }

    fn ffi_call_return_type(
        &self,
        sig: &FfiSignature,
    ) -> Result<Option<BasicTypeEnum<'ctx>>, String> {
        match (&sig.error_abi, &sig.ret) {
            (FfiErrorAbi::ThawResult, HirType::Void) => Ok(Some(
                self.context
                    .struct_type(
                        &[self.context.ptr_type(AddressSpace::default()).into()],
                        false,
                    )
                    .into(),
            )),
            (FfiErrorAbi::ThawResult, ret) => {
                let value_type = self.ffi_return_type(
                    ret,
                    sig.return_string_abi,
                    sig.aggregate_return_abi,
                    sig.aggregate_return_layout.as_ref(),
                )?;
                if let Some(layout) = &sig.aggregate_return_layout {
                    Ok(Some(
                        self.ffi_explicit_result_type(value_type, layout)?.0.into(),
                    ))
                } else {
                    Ok(Some(
                        self.context
                            .struct_type(
                                &[
                                    value_type,
                                    self.context.ptr_type(AddressSpace::default()).into(),
                                ],
                                false,
                            )
                            .into(),
                    ))
                }
            }
            (FfiErrorAbi::Direct, HirType::Void) => Ok(None),
            (FfiErrorAbi::Direct, ret) => {
                if let Some(layout) = sig
                    .aggregate_return_layout
                    .as_ref()
                    .filter(|layout| !layout.register_classes.is_empty())
                {
                    return Ok(Some(self.ffi_register_return_type(layout).into()));
                }
                self.ffi_return_type(
                    ret,
                    sig.return_string_abi,
                    sig.aggregate_return_abi,
                    sig.aggregate_return_layout.as_ref(),
                )
                .map(Some)
            }
        }
    }

    fn ffi_register_return_type(
        &self,
        layout: &FfiAggregateLayout,
    ) -> inkwell::types::StructType<'ctx> {
        let fields = layout
            .register_classes
            .iter()
            .map(|class| match class {
                FfiRegisterClass::Integer => self.context.i64_type().into(),
                FfiRegisterClass::Sse => self.context.f64_type().into(),
            })
            .collect::<Vec<BasicTypeEnum<'ctx>>>();
        self.context.struct_type(&fields, false)
    }

    fn ffi_return_type(
        &self,
        ty: &HirType,
        string_abi: FfiStringAbi,
        aggregate_abi: FfiAggregateAbi,
        aggregate_layout: Option<&FfiAggregateLayout>,
    ) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            HirType::Str if string_abi == FfiStringAbi::PointerLength => Ok(self
                .context
                .struct_type(
                    &[
                        self.context.ptr_type(AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ],
                    false,
                )
                .into()),
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue
                )
                    && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                Ok(self
                    .context
                    .struct_type(
                        &[
                            self.context.ptr_type(AddressSpace::default()).into(),
                            self.context.i64_type().into(),
                        ],
                        aggregate_abi == FfiAggregateAbi::Packed,
                    )
                    .into())
            }
            HirType::Object(fields) if aggregate_abi != FfiAggregateAbi::Internal => {
                if let Some(layout) = aggregate_layout {
                    return self
                        .ffi_explicit_object_type(fields, layout)
                        .map(|(ty, _)| ty.into());
                }
                let fields = fields
                    .iter()
                    .map(|(name, ty)| {
                        self.ffi_return_type(ty, FfiStringAbi::NullTerminated, aggregate_abi, None)
                            .map_err(|error| format!("FFI object field `{name}`: {error}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(self
                    .context
                    .struct_type(&fields, aggregate_abi == FfiAggregateAbi::Packed)
                    .into())
            }
            other => self.basic_type(other),
        }
    }

    fn ffi_explicit_object_type(
        &self,
        fields: &[(String, HirType)],
        layout: &FfiAggregateLayout,
    ) -> Result<(inkwell::types::StructType<'ctx>, Vec<ExplicitFfiField>), String> {
        let mut native_fields = Vec::new();
        let mut logical_indices = Vec::with_capacity(fields.len());
        let mut cursor = 0_u64;
        let mut shared_bit_storage = None;
        for (index, ((name, field_ty), offset)) in
            fields.iter().zip(&layout.field_offsets).enumerate()
        {
            if let Some(bitfield) = &layout.field_bitfields[index] {
                let shared_index =
                    shared_bit_storage.and_then(|(shared_offset, shared_bytes, native_index)| {
                        (shared_offset == *offset && shared_bytes == bitfield.storage_bytes)
                            .then_some(native_index)
                    });
                let native_index = if let Some(native_index) = shared_index {
                    native_index
                } else {
                    if *offset < cursor {
                        return Err(format!(
                            "FFI bitfield `{name}` does not share its declared storage unit"
                        ));
                    }
                    if *offset > cursor {
                        let padding = u32::try_from(*offset - cursor).map_err(|_| {
                            "FFI aggregate padding exceeds LLVM's array limit".to_owned()
                        })?;
                        native_fields.push(self.context.i8_type().array_type(padding).into());
                    }
                    let native_index = native_fields.len() as u32;
                    native_fields.push(
                        self.context
                            .custom_width_int_type(
                                NonZeroU32::new(u32::from(bitfield.storage_bytes) * 8)
                                    .expect("validated bitfield storage is non-zero"),
                            )
                            .map_err(str::to_owned)?
                            .into(),
                    );
                    cursor = offset + u64::from(bitfield.storage_bytes);
                    shared_bit_storage = Some((*offset, bitfield.storage_bytes, native_index));
                    native_index
                };
                logical_indices.push(ExplicitFfiField {
                    native_index,
                    bitfield: Some(bitfield.clone()),
                });
                continue;
            }
            shared_bit_storage = None;
            if *offset > cursor {
                let padding = u32::try_from(*offset - cursor)
                    .map_err(|_| "FFI aggregate padding exceeds LLVM's array limit".to_owned())?;
                native_fields.push(self.context.i8_type().array_type(padding).into());
            }
            logical_indices.push(ExplicitFfiField {
                native_index: native_fields.len() as u32,
                bitfield: None,
            });
            let native_field = if *field_ty == HirType::Bool {
                self.context.i8_type().into()
            } else {
                self.ffi_return_type(
                    field_ty,
                    FfiStringAbi::NullTerminated,
                    FfiAggregateAbi::Portable,
                    layout.field_layouts[index].as_deref(),
                )
                .map_err(|error| format!("FFI object field `{name}`: {error}"))?
            };
            native_fields.push(native_field);
            let field_size = layout.field_layouts[index].as_deref().map_or_else(
                || Self::ffi_object_field_layout(field_ty, FfiAggregateAbi::Portable).0,
                |layout| layout.size,
            );
            cursor = offset + field_size;
        }
        if layout.size > cursor {
            let padding = u32::try_from(layout.size - cursor)
                .map_err(|_| "FFI aggregate tail padding exceeds LLVM's array limit".to_owned())?;
            native_fields.push(self.context.i8_type().array_type(padding).into());
        }
        Ok((
            self.context.struct_type(&native_fields, true),
            logical_indices,
        ))
    }

    fn ffi_explicit_result_type(
        &self,
        value_type: BasicTypeEnum<'ctx>,
        layout: &FfiAggregateLayout,
    ) -> Result<(inkwell::types::StructType<'ctx>, (u32, u32)), String> {
        let pointer_size = 8_u64;
        let error_offset = Self::align_to(layout.size, pointer_size);
        let result_alignment = u64::from(layout.alignment).max(pointer_size);
        let result_size = Self::align_to(error_offset + pointer_size, result_alignment);
        let mut fields = vec![value_type];
        if error_offset > layout.size {
            let padding = u32::try_from(error_offset - layout.size)
                .map_err(|_| "FFI result padding exceeds LLVM's array limit".to_owned())?;
            fields.push(self.context.i8_type().array_type(padding).into());
        }
        let error_index = fields.len() as u32;
        fields.push(self.context.ptr_type(AddressSpace::default()).into());
        if result_size > error_offset + pointer_size {
            let padding = u32::try_from(result_size - error_offset - pointer_size)
                .map_err(|_| "FFI result tail padding exceeds LLVM's array limit".to_owned())?;
            fields.push(self.context.i8_type().array_type(padding).into());
        }
        Ok((self.context.struct_type(&fields, true), (0, error_index)))
    }

    fn ffi_result_field_indices(&self, sig: &FfiSignature) -> Result<(u32, u32), String> {
        let Some(layout) = &sig.aggregate_return_layout else {
            return Ok((0, 1));
        };
        let value_type = self.ffi_return_type(
            &sig.ret,
            sig.return_string_abi,
            sig.aggregate_return_abi,
            Some(layout),
        )?;
        self.ffi_explicit_result_type(value_type, layout)
            .map(|(_, indices)| indices)
    }

    /// The real C ABI parameter list a Fast path native symbol is declared
    /// with, for the given `.d.ts`-derived HIR parameter types --
    /// docs/design/bridge.md section 5's "Marshal コード生成", the part
    /// left explicitly unimplemented when Fast path FFI calls were first
    /// added (see docs/design/registry.md section 18, "実際の C ABI...
    /// に合わせた Marshal アダプタ生成"). Thaw's own internal
    /// representation for `Array`/`Object` (a single opaque pointer to an
    /// arena buffer with Thaw-specific layout) is almost never what a real
    /// native library's C signature expects, so those two are expanded
    /// here into the shape a real C API actually tends to use -- this
    /// list, and `compile_ffi_call`'s argument-building loop, must stay in
    /// lockstep (each match arm here has a matching arm there).
    ///
    /// - `number[]` -> two C parameters, `(const double*, int64_t len)` --
    ///   the near-universal C convention for passing an array, as opposed
    ///   to Thaw's own single-pointer `[i64 len][f64 elements...]` buffer.
    /// - `{ x: number; ... }` -> one C parameter *per field*, in
    ///   declaration order (`distance(x1, y1, x2, y2)` rather than a
    ///   single struct pointer) -- Fast path classification only accepts
    ///   `number` object fields (docs/design/bridge.md section 4.1), so
    ///   this is always one `double` per field. A real by-value C struct
    ///   parameter would need to replicate the target's own struct-passing
    ///   ABI (register-class rules, padding, ...), which isn't attempted
    ///   here; a native library that genuinely wants a struct by value
    ///   needs its own flat-field C wrapper, the same way many real C APIs
    ///   already choose to expose one.
    /// - everything else -- unchanged, one C parameter each.
    fn ffi_param_types(
        &self,
        params: &[HirType],
        string_abis: &[FfiStringAbi],
    ) -> Result<Vec<BasicMetadataTypeEnum<'ctx>>, String> {
        let mut out = Vec::with_capacity(params.len());
        for (index, ty) in params.iter().enumerate() {
            match ty {
                HirType::Str if string_abis.get(index) == Some(&FfiStringAbi::PointerLength) => {
                    out.push(self.context.ptr_type(AddressSpace::default()).into());
                    out.push(self.context.i64_type().into());
                }
                other => self.append_ffi_param_type(other, &mut out)?,
            }
        }
        Ok(out)
    }

    fn append_ffi_param_type(
        &self,
        ty: &HirType,
        out: &mut Vec<BasicMetadataTypeEnum<'ctx>>,
    ) -> Result<(), String> {
        match ty {
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue
                ) =>
            {
                out.push(self.context.ptr_type(AddressSpace::default()).into());
                out.push(self.context.i64_type().into());
            }
            HirType::Object(fields) => {
                for (name, field) in fields {
                    self.append_ffi_param_type(field, out)
                        .map_err(|error| format!("FFI object field `{name}`: {error}"))?;
                }
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                out.push(self.context.i8_type().into());
                self.append_ffi_param_type(payload, out)?;
            }
            HirType::Nullish(payload) => {
                out.push(self.context.i8_type().into());
                self.append_ffi_param_type(payload, out)?;
            }
            other => out.push(self.basic_type(other).map(BasicMetadataTypeEnum::from)?),
        }
        Ok(())
    }
}
