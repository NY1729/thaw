#[derive(Clone, Debug, PartialEq)]
struct FfiMetadata {
    error_abi: thaw_hir::FfiErrorAbi,
    return_ownership: thaw_hir::FfiOwnership,
    error_ownership: thaw_hir::FfiOwnership,
    param_string_abis: Option<Vec<thaw_hir::FfiStringAbi>>,
    return_string_abi: thaw_hir::FfiStringAbi,
    calling_convention: thaw_hir::FfiCallingConvention,
    aggregate_return_abi: thaw_hir::FfiAggregateAbi,
    aggregate_return_layout: Option<thaw_hir::FfiAggregateLayout>,
    variadic_abi: thaw_hir::FfiVariadicAbi,
}

fn parse_string_abi(
    value: &str,
    symbol: &str,
    path: &Path,
) -> Result<thaw_hir::FfiStringAbi, String> {
    match value {
        "null-terminated" => Ok(thaw_hir::FfiStringAbi::NullTerminated),
        "pointer-length" => Ok(thaw_hir::FfiStringAbi::PointerLength),
        other => Err(format!(
            "unknown string ABI `{other}` for `{symbol}` in `{}`",
            path.display()
        )),
    }
}

fn parse_ffi_ownership(
    entry: &serde_json::Value,
    ownership_key: &str,
    destroy_key: &str,
    fallback_destroy_key: Option<&str>,
    symbol: &str,
    path: &Path,
) -> Result<thaw_hir::FfiOwnership, String> {
    let spelling = entry
        .get(ownership_key)
        .and_then(|value| value.as_str())
        .unwrap_or("borrowed");
    let destroy = entry
        .get(destroy_key)
        .or_else(|| fallback_destroy_key.and_then(|key| entry.get(key)))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    match spelling {
        "borrowed" => Ok(thaw_hir::FfiOwnership::Borrowed),
        "owned" => destroy
            .map(|destroy| thaw_hir::FfiOwnership::Owned { destroy })
            .ok_or_else(|| {
                format!(
                    "FFI metadata for `{symbol}` in `{}` needs `{destroy_key}` for `{ownership_key}: owned`",
                    path.display()
                )
            }),
        "arena-copy" => Ok(thaw_hir::FfiOwnership::ArenaCopy { destroy }),
        other => Err(format!(
            "unknown {ownership_key} `{other}` for `{symbol}` in `{}`",
            path.display()
        )),
    }
}

fn parse_ffi_aggregate_layout(
    value: &serde_json::Value,
    symbol: &str,
    path: &Path,
    root: bool,
) -> Result<thaw_hir::FfiAggregateLayout, String> {
    let object = value.as_object().ok_or_else(|| {
        format!(
            "FFI metadata for `{symbol}` in `{}` requires aggregate layouts to be objects",
            path.display()
        )
    })?;
    let field_offsets = object
        .get("fieldOffsets")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires `fieldOffsets` to be an array",
                path.display()
            )
        })?
        .iter()
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                format!(
                    "FFI metadata for `{symbol}` in `{}` requires non-negative integer field offsets",
                    path.display()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let field_layouts = object
        .get("fieldLayouts")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` requires `fieldLayouts` to be an array",
                        path.display()
                    )
                })?
                .iter()
                .map(|value| {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        parse_ffi_aggregate_layout(value, symbol, path, false)
                            .map(Box::new)
                            .map(Some)
                    }
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![None; field_offsets.len()]);
    let field_bitfields = object
        .get("bitFields")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` requires `bitFields` to be an array",
                        path.display()
                    )
                })?
                .iter()
                .map(|value| {
                    if value.is_null() {
                        return Ok(None);
                    }
                    let bitfield = value.as_object().ok_or_else(|| {
                        format!(
                            "FFI metadata for `{symbol}` in `{}` requires bitfield entries to be objects or null",
                            path.display()
                        )
                    })?;
                    let bit_offset = bitfield
                        .get("bitOffset")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| {
                            format!(
                                "FFI metadata for `{symbol}` in `{}` requires an 8-bit integer `bitOffset`",
                                path.display()
                            )
                        })?;
                    let bit_width = match bitfield.get("bitWidth") {
                        Some(value) => value
                            .as_u64()
                            .and_then(|value| u8::try_from(value).ok())
                            .ok_or_else(|| {
                                format!(
                                    "FFI metadata for `{symbol}` in `{}` requires an 8-bit integer `bitWidth`",
                                    path.display()
                                )
                            })?,
                        None => 1,
                    };
                    let storage_bytes = bitfield
                        .get("storageBytes")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(|| {
                            format!(
                                "FFI metadata for `{symbol}` in `{}` requires an 8-bit integer `storageBytes`",
                                path.display()
                            )
                        })?;
                    let signed = bitfield
                        .get("signed")
                        .map(|value| {
                            value.as_bool().ok_or_else(|| {
                                format!(
                                    "FFI metadata for `{symbol}` in `{}` requires bitfield `signed` to be a boolean",
                                    path.display()
                                )
                            })
                        })
                        .transpose()?
                        .unwrap_or(false);
                    Ok(Some(thaw_hir::FfiBitFieldLayout {
                        bit_offset,
                        bit_width,
                        storage_bytes,
                        signed,
                    }))
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![None; field_offsets.len()]);
    let register_classes = object
        .get("registerClasses")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` requires `registerClasses` to be an array",
                        path.display()
                    )
                })?
                .iter()
                .map(|value| match value.as_str() {
                    Some("integer") => Ok(thaw_hir::FfiRegisterClass::Integer),
                    Some("sse") => Ok(thaw_hir::FfiRegisterClass::Sse),
                    _ => Err(format!(
                        "FFI metadata for `{symbol}` in `{}` has an unknown register class",
                        path.display()
                    )),
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let size = object
        .get("size")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires an integer aggregate layout `size`",
                path.display()
            )
        })?;
    let alignment = object
        .get("alignment")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires a 32-bit integer aggregate layout `alignment`",
                path.display()
            )
        })?;
    let indirect = match object.get("indirect") {
        Some(value) => value.as_bool().ok_or_else(|| {
            format!(
                "FFI metadata for `{symbol}` in `{}` requires aggregate layout `indirect` to be a boolean",
                path.display()
            )
        })?,
        None if root => {
            return Err(format!(
                "FFI metadata for `{symbol}` in `{}` requires `aggregateReturnLayout.indirect`",
                path.display()
            ))
        }
        None => false,
    };
    Ok(thaw_hir::FfiAggregateLayout {
        field_offsets,
        field_layouts,
        field_bitfields,
        register_classes,
        size,
        alignment,
        indirect,
    })
}

fn read_ffi_metadata(
    paths: &[PathBuf],
) -> Result<std::collections::HashMap<String, FfiMetadata>, String> {
    let mut configured = std::collections::HashMap::new();
    for path in paths {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
        let document: serde_json::Value = serde_json::from_str(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", path.display()))?;
        let version = document.get("version").and_then(|value| value.as_u64());
        if !matches!(version, Some(1..=4)) {
            return Err(format!(
                "`{}` must declare FFI metadata version 1, 2, 3 or 4",
                path.display()
            ));
        }
        let functions = document
            .get("functions")
            .and_then(|value| value.as_object())
            .ok_or_else(|| format!("`{}` needs a `functions` object", path.display()))?;
        for (symbol, entry) in functions {
            let spelling = entry
                .get("errorAbi")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    format!(
                        "FFI metadata for `{symbol}` in `{}` needs `errorAbi`",
                        path.display()
                    )
                })?;
            let abi = match spelling {
                "direct" => thaw_hir::FfiErrorAbi::Direct,
                "thaw-result" => thaw_hir::FfiErrorAbi::ThawResult,
                other => {
                    return Err(format!(
                        "unknown errorAbi `{other}` for `{symbol}` in `{}`",
                        path.display()
                    ))
                }
            };
            let metadata = FfiMetadata {
                error_abi: abi,
                return_ownership: if matches!(version, Some(2..=4)) {
                    parse_ffi_ownership(
                        entry,
                        "returnOwnership",
                        "returnDestroy",
                        Some("destroy"),
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiOwnership::Borrowed
                },
                error_ownership: if matches!(version, Some(2..=4)) {
                    parse_ffi_ownership(
                        entry,
                        "errorOwnership",
                        "errorDestroy",
                        None,
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiOwnership::Borrowed
                },
                param_string_abis: if matches!(version, Some(3 | 4)) {
                    entry
                        .get("parameterStringAbis")
                        .map(|value| {
                            value
                                .as_array()
                                .ok_or_else(|| {
                                    format!(
                                        "FFI metadata for `{symbol}` in `{}` requires `parameterStringAbis` to be an array",
                                        path.display()
                                    )
                                })?
                                .iter()
                                .map(|value| {
                                    let value = value.as_str().ok_or_else(|| {
                                        format!(
                                            "FFI metadata for `{symbol}` in `{}` requires string ABI names",
                                            path.display()
                                        )
                                    })?;
                                    parse_string_abi(value, symbol, path)
                                })
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .transpose()?
                } else {
                    None
                },
                return_string_abi: if matches!(version, Some(3 | 4)) {
                    parse_string_abi(
                        entry
                            .get("returnStringAbi")
                            .and_then(|value| value.as_str())
                            .unwrap_or("null-terminated"),
                        symbol,
                        path,
                    )?
                } else {
                    thaw_hir::FfiStringAbi::NullTerminated
                },
                calling_convention: if matches!(version, Some(3 | 4)) {
                    match entry
                        .get("callingConvention")
                        .and_then(|value| value.as_str())
                        .unwrap_or("c")
                    {
                        "c" => thaw_hir::FfiCallingConvention::C,
                        "fast" => thaw_hir::FfiCallingConvention::Fast,
                        "cold" => thaw_hir::FfiCallingConvention::Cold,
                        other => {
                            return Err(format!(
                                "unknown calling convention `{other}` for `{symbol}` in `{}`",
                                path.display()
                            ))
                        }
                    }
                } else {
                    thaw_hir::FfiCallingConvention::C
                },
                aggregate_return_abi: if matches!(version, Some(3 | 4)) {
                    match entry
                        .get("aggregateReturnAbi")
                        .and_then(|value| value.as_str())
                        .unwrap_or("internal")
                    {
                        "internal" => thaw_hir::FfiAggregateAbi::Internal,
                        "portable" => thaw_hir::FfiAggregateAbi::Portable,
                        "packed" => thaw_hir::FfiAggregateAbi::Packed,
                        other => {
                            return Err(format!(
                                "unknown aggregate return ABI `{other}` for `{symbol}` in `{}`",
                                path.display()
                            ))
                        }
                    }
                } else {
                    thaw_hir::FfiAggregateAbi::Internal
                },
                aggregate_return_layout: if version == Some(4) {
                    entry
                        .get("aggregateReturnLayout")
                        .map(|value| parse_ffi_aggregate_layout(value, symbol, path, true))
                        .transpose()?
                } else {
                    None
                },
                variadic_abi: if version == Some(4) {
                    match entry
                        .get("variadicAbi")
                        .and_then(|value| value.as_str())
                        .unwrap_or("native")
                    {
                        "native" => thaw_hir::FfiVariadicAbi::Native,
                        "i32" => thaw_hir::FfiVariadicAbi::I32,
                        "i64" => thaw_hir::FfiVariadicAbi::I64,
                        "u32" => thaw_hir::FfiVariadicAbi::U32,
                        "u64" => thaw_hir::FfiVariadicAbi::U64,
                        other => {
                            return Err(format!(
                                "unknown variadic ABI `{other}` for `{symbol}` in `{}`",
                                path.display()
                            ))
                        }
                    }
                } else {
                    thaw_hir::FfiVariadicAbi::Native
                },
            };
            if let Some(previous) = configured.insert(symbol.clone(), metadata.clone()) {
                if previous != metadata {
                    return Err(format!("conflicting FFI metadata for `{symbol}`"));
                }
            }
        }
    }
    Ok(configured)
}
