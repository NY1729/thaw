#[test]
fn qualifier_identifier_passes_through_unscoped_names() {
    assert_eq!(qualifier_identifier("qs"), "qs");
    assert_eq!(qualifier_identifier("left-pad"), "left-pad");
}

#[test]
fn qualifier_identifier_uses_the_last_segment_of_a_scoped_name() {
    assert_eq!(qualifier_identifier("@hapi/hoek"), "hoek");
    assert_eq!(qualifier_identifier("@babel/core"), "core");
}

#[test]
fn qualifier_identifiers_disambiguate_equal_scoped_package_tails() {
    let qualifiers = package_qualifier_identifiers(["@foo/utils", "@bar/utils", "@hapi/hoek"]);
    assert_eq!(qualifiers["@foo/utils"], "_foo_utils");
    assert_eq!(qualifiers["@bar/utils"], "_bar_utils");
    assert_eq!(qualifiers["@hapi/hoek"], "hoek");
}

#[test]
fn qualifier_identifiers_remain_unique_after_sanitization() {
    let qualifiers =
        package_qualifier_identifiers(["@foo-bar/utils", "@foo_bar/utils", "_foo_bar_utils"]);
    let unique = qualifiers
        .values()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), qualifiers.len());
    assert!(qualifiers.values().all(|qualifier| qualifier
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')));
}

#[test]
fn sanitize_identifier_replaces_non_alphanumerics() {
    assert_eq!(sanitize_identifier("qs"), "qs");
    assert_eq!(sanitize_identifier("@hapi/hoek"), "_hapi_hoek");
    assert_eq!(sanitize_identifier("left-pad"), "left_pad");
}

#[test]
fn reads_commonjs_export_assignment_target() {
    let declarations = "declare function weak<T extends object>(value: T): T; export = weak;";
    assert_eq!(
        commonjs_export_name(declarations).unwrap(),
        Some("weak".to_string())
    );
}

#[test]
fn reads_versioned_ffi_error_abi_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-metadata-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
        &path,
        r#"{"version":1,"functions":{"externalRead":{"errorAbi":"thaw-result"}}}"#,
    )
    .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["externalRead"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::ThawResult,
            return_ownership: thaw_hir::FfiOwnership::Borrowed,
            error_ownership: thaw_hir::FfiOwnership::Borrowed,
            param_string_abis: None,
            return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
            calling_convention: thaw_hir::FfiCallingConvention::C,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_unknown_ffi_metadata_versions_and_abis() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-invalid-ffi-metadata-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let version = dir.join("version.json");
    std::fs::write(&version, r#"{"version":5,"functions":{}}"#).unwrap();
    assert!(read_ffi_metadata(&[version])
        .unwrap_err()
        .contains("version 1"));
    let abi = dir.join("abi.json");
    std::fs::write(
        &abi,
        r#"{"version":1,"functions":{"f":{"errorAbi":"errno"}}}"#,
    )
    .unwrap();
    assert!(read_ffi_metadata(&[abi])
        .unwrap_err()
        .contains("unknown errorAbi"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_two_ffi_ownership_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-ownership-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":2,"functions":{"read":{"errorAbi":"thaw-result","returnOwnership":"owned","destroy":"free_read","errorOwnership":"arena-copy","errorDestroy":"free_error"}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["read"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::ThawResult,
            return_ownership: thaw_hir::FfiOwnership::Owned {
                destroy: "free_read".into()
            },
            error_ownership: thaw_hir::FfiOwnership::ArenaCopy {
                destroy: Some("free_error".into())
            },
            param_string_abis: None,
            return_string_abi: thaw_hir::FfiStringAbi::NullTerminated,
            calling_convention: thaw_hir::FfiCallingConvention::C,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_three_string_abi_metadata() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-ffi-string-abi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":3,"functions":{"slice":{"errorAbi":"direct","parameterStringAbis":["pointer-length"],"returnStringAbi":"pointer-length","callingConvention":"fast","aggregateReturnAbi":"portable"},"record":{"errorAbi":"direct","aggregateReturnAbi":"packed"}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["slice"],
        FfiMetadata {
            error_abi: thaw_hir::FfiErrorAbi::Direct,
            return_ownership: thaw_hir::FfiOwnership::Borrowed,
            error_ownership: thaw_hir::FfiOwnership::Borrowed,
            param_string_abis: Some(vec![thaw_hir::FfiStringAbi::PointerLength]),
            return_string_abi: thaw_hir::FfiStringAbi::PointerLength,
            calling_convention: thaw_hir::FfiCallingConvention::Fast,
            aggregate_return_abi: thaw_hir::FfiAggregateAbi::Portable,
            aggregate_return_layout: None,
            variadic_abi: thaw_hir::FfiVariadicAbi::Native,
        }
    );
    assert_eq!(
        metadata["record"].aggregate_return_abi,
        thaw_hir::FfiAggregateAbi::Packed
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_version_four_explicit_aggregate_layout() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-ffi-aggregate-layout-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ffi.json");
    std::fs::write(
            &path,
            r#"{"version":4,"functions":{"record":{"errorAbi":"direct","aggregateReturnAbi":"portable","aggregateReturnLayout":{"fieldOffsets":[0,16],"fieldLayouts":[null,{"fieldOffsets":[0],"size":8,"alignment":8}],"bitFields":[{"bitOffset":2,"bitWidth":5,"storageBytes":1,"signed":true},null],"size":32,"alignment":32,"indirect":true}},"register":{"errorAbi":"direct","aggregateReturnAbi":"portable","variadicAbi":"i64","aggregateReturnLayout":{"fieldOffsets":[0,8],"size":16,"alignment":8,"indirect":false,"registerClasses":["integer","sse"]}}}}"#,
        )
        .unwrap();
    let metadata = read_ffi_metadata(&[path]).unwrap();
    assert_eq!(
        metadata["record"].aggregate_return_layout,
        Some(thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![
                None,
                Some(Box::new(thaw_hir::FfiAggregateLayout {
                    field_offsets: vec![0],
                    field_layouts: vec![None],
                    field_bitfields: vec![None],
                    register_classes: vec![],
                    size: 8,
                    alignment: 8,
                    indirect: false,
                })),
            ],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 1,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        })
    );
    assert_eq!(
        metadata["register"].variadic_abi,
        thaw_hir::FfiVariadicAbi::I64
    );
    assert_eq!(
        metadata["register"]
            .aggregate_return_layout
            .as_ref()
            .unwrap()
            .register_classes,
        vec![
            thaw_hir::FfiRegisterClass::Integer,
            thaw_hir::FfiRegisterClass::Sse,
        ]
    );
    let _ = std::fs::remove_dir_all(dir);
}
