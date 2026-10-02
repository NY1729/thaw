#[test]
fn detects_dynamic_interpreters_in_elf_outputs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-elf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app");
    let mut elf = vec![0_u8; 120];
    elf[..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
    elf[54..56].copy_from_slice(&56_u16.to_le_bytes());
    elf[56..58].copy_from_slice(&1_u16.to_le_bytes());
    elf[64..68].copy_from_slice(&3_u32.to_le_bytes());
    std::fs::write(&path, &elf).unwrap();
    assert!(elf_has_program_interpreter(&path).unwrap());

    elf[64..68].copy_from_slice(&1_u32.to_le_bytes());
    std::fs::write(&path, &elf).unwrap();
    assert!(!elf_has_program_interpreter(&path).unwrap());
    elf[5] = 2;
    elf[32..40].copy_from_slice(&64_u64.to_be_bytes());
    elf[54..56].copy_from_slice(&56_u16.to_be_bytes());
    elf[56..58].copy_from_slice(&1_u16.to_be_bytes());
    elf[64..68].copy_from_slice(&3_u32.to_be_bytes());
    std::fs::write(&path, &elf).unwrap();
    assert!(elf_has_program_interpreter(&path).unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn static_toolchain_check_is_actionable_or_complete() {
    if let Err(error) = ensure_static_system_libraries() {
        assert!(error.contains("--static"), "{error}");
        assert!(error.contains("glibc-static"), "{error}");
    }
}

#[test]
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn specialized_jit_runs_without_quickjs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-jit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    let string_symbol = "expr:s0,strlen:test"
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let concat_symbol = "expr:t21,s0,concat:test"
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::write(
        &source,
        format!("declare function __thaw_typed_jit_6164643a74657374(left: number, right: number): number;\ndeclare function __thaw_typed_jit_{string_symbol}(value: string): number;\ndeclare function __thaw_typed_jit_{concat_symbol}(value: string): string;\nfunction main(): void {{ console.log(__thaw_typed_jit_6164643a74657374(20, 22) + __thaw_typed_jit_{string_symbol}(__thaw_typed_jit_{concat_symbol}('😀')) - 3); }}\n"),
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn tagged_jit_union_runs_without_quickjs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-jit-union-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    let operation = format!(
        "expr:a0,asbool,if,c{:016x},tagnum,else,t68656c6c6f,tagstr,end:tagged-union",
        42.0f64.to_bits()
    );
    let symbol = operation
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::write(
        &source,
        format!(
            "declare function __thaw_typed_jit_{symbol}(numberResult: boolean): number | string;\nfunction main(): void {{ const numberResult = __thaw_typed_jit_{symbol}(true); if (typeof numberResult === 'number') {{ console.log(numberResult + 1); }} const stringResult = __thaw_typed_jit_{symbol}(false); if (typeof stringResult === 'string') {{ console.log(stringResult.toUpperCase()); }} }}\n"
        ),
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "43\nHELLO\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mixed_scalar_array_jit_union_normalizes_only_the_array_tag() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-jit-scalar-array-union-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    let operation = format!(
        "expr:a0,asbool,if,arrayempty,c{:016x},rnappend,tagrn,else,c{:016x},tagnum,end:mixed-union",
        7.0f64.to_bits(),
        42.0f64.to_bits()
    );
    let symbol = operation
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::write(
        &source,
        format!(
            "declare function __thaw_typed_jit_{symbol}(array: boolean): number[] | number;\nfunction main(): void {{ const scalar = __thaw_typed_jit_{symbol}(false); if (typeof scalar === 'number') console.log(scalar); console.log(Array.isArray(__thaw_typed_jit_{symbol}(true))); }}\n"
        ),
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\ntrue\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn builds_and_runs_a_fully_static_elf() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-static-elf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
    build_with_link_mode(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    assert!(!elf_has_program_interpreter(&output).unwrap());
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["packages"], serde_json::json!([]));
    assert_eq!(manifest["quickjs"], false);
    assert_eq!(manifest["napi"], false);
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn static_build_rejects_dynamic_napi_addons() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-static-napi-{}", std::process::id()));
    let package = dir.join("registry/native-add");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("native.node"),
        b"not loaded during compilation",
    )
    .unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        "import { add } from \"native-add\"; function main(): void {}\n",
    )
    .unwrap();
    let error = build_with_link_mode(
        &source,
        &dir.join("app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap_err();
    assert!(error.contains("N-API addon"), "{error}");
    assert!(error.contains("native.a"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(target_os = "linux")]
fn fully_static_binary_runs_in_an_isolated_container_when_enabled() {
    if std::env::var("THAW_RUN_CONTAINER_INTEGRATION").as_deref() != Ok("1")
        || ensure_static_system_libraries().is_err()
    {
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-static-container-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(&source, "function main(): void { console.log(42); }\n").unwrap();
    build_with_link_mode(
        &source,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    let result = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "--security-opt",
            "label=disable",
            "-v",
        ])
        .arg(format!("{}:/app:ro", output.display()))
        .args(["registry.fedoraproject.org/fedora:41", "/app"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

fn artifact_section_fixture(big_endian: bool, metadata: &[u8]) -> Vec<u8> {
    fn put16(bytes: &mut [u8], offset: usize, value: u16, big: bool) {
        bytes[offset..offset + 2].copy_from_slice(&if big { value.to_be_bytes() } else { value.to_le_bytes() });
    }
    fn put32(bytes: &mut [u8], offset: usize, value: u32, big: bool) {
        bytes[offset..offset + 4].copy_from_slice(&if big { value.to_be_bytes() } else { value.to_le_bytes() });
    }
    fn put64(bytes: &mut [u8], offset: usize, value: u64, big: bool) {
        bytes[offset..offset + 8].copy_from_slice(&if big { value.to_be_bytes() } else { value.to_le_bytes() });
    }
    let names = b"\0.shstrtab\0.thaw.artifact\0";
    let decoy = b"THAW_ARTIFACT_V1:{\"wrong\":true}\0";
    let names_offset = 64 + decoy.len();
    let metadata_offset = names_offset + names.len();
    let section_offset = metadata_offset + metadata.len();
    let mut elf = vec![0; section_offset + 3 * 64];
    elf[..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = if big_endian { 2 } else { 1 };
    put16(&mut elf, 18, if big_endian { 0xb7 } else { 0x3e }, big_endian);
    put64(&mut elf, 40, section_offset as u64, big_endian);
    put16(&mut elf, 58, 64, big_endian);
    put16(&mut elf, 60, 3, big_endian);
    put16(&mut elf, 62, 1, big_endian);
    elf[64..names_offset].copy_from_slice(decoy);
    elf[names_offset..metadata_offset].copy_from_slice(names);
    elf[metadata_offset..section_offset].copy_from_slice(metadata);
    let strings = section_offset + 64;
    put32(&mut elf, strings, 1, big_endian);
    put32(&mut elf, strings + 4, 3, big_endian);
    put64(&mut elf, strings + 24, names_offset as u64, big_endian);
    put64(&mut elf, strings + 32, names.len() as u64, big_endian);
    let artifact = section_offset + 128;
    put32(&mut elf, artifact, 11, big_endian);
    put32(&mut elf, artifact + 4, 1, big_endian);
    put64(&mut elf, artifact + 24, metadata_offset as u64, big_endian);
    put64(&mut elf, artifact + 32, metadata.len() as u64, big_endian);
    elf
}

#[test]
fn artifact_manifest_uses_only_the_exact_elf_section_in_both_byte_orders() {
    let payload = b"THAW_ARTIFACT_V1:{\"packages\":[\"correct\"],\"quickjs\":false}\0";
    for big in [false, true] {
        let elf = artifact_section_fixture(big, payload);
        let manifest = artifact_manifest_from_bytes(&elf).unwrap();
        assert_eq!(manifest["packages"][0], "correct");
        let mut invalid_decoy = elf.clone();
        let decoy_json = 64 + ARTIFACT_MARKER.len();
        invalid_decoy[decoy_json] = b'!';
        assert_eq!(artifact_manifest_from_bytes(&invalid_decoy).unwrap()["packages"][0], "correct");
        let endian = elf_endian(&elf).unwrap();
        assert_eq!(endian.u16(&elf[18..]).unwrap(), if big { 0xb7 } else { 0x3e });
    }
}

#[test]
fn artifact_manifest_rejects_missing_duplicate_and_malformed_sections() {
    let payload = b"THAW_ARTIFACT_V1:{}\0";
    let valid = artifact_section_fixture(false, payload);
    let section = valid.len() - 64;
    let mut elf = valid.clone();
    elf[section..section + 4].copy_from_slice(&0_u32.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).unwrap_err().contains("does not contain"));
    let mut elf = valid.clone();
    elf[section + 4..section + 8].copy_from_slice(&8_u32.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).unwrap_err().contains("PROGBITS"));
    let mut elf = valid.clone();
    elf[60..62].copy_from_slice(&4_u16.to_le_bytes());
    elf.extend_from_slice(&valid[section..]);
    assert!(artifact_manifest_from_bytes(&elf).unwrap_err().contains("duplicate"));
    let mut elf = valid.clone();
    elf[section + 24..section + 32].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).is_err());
    let mut elf = valid.clone();
    elf[section + 32..section + 40].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).is_err());
    let mut elf = valid.clone();
    elf[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).is_err());
    let mut elf = valid.clone();
    elf[62..64].copy_from_slice(&0xffff_u16.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).is_err());
    let mut elf = valid.clone();
    let strings = section - 64;
    elf[strings + 32..strings + 40].copy_from_slice(&1_u64.to_le_bytes());
    assert!(artifact_manifest_from_bytes(&elf).is_err());
    for payload in [b"THAW_ARTIFACT_V2:{}\0".as_slice(), b"THAW_ARTIFACT_V1:{}".as_slice(), b"THAW_ARTIFACT_V1:{bad}\0".as_slice()] {
        assert!(artifact_manifest_from_bytes(&artifact_section_fixture(false, payload)).is_err());
    }
}

#[test]
fn built_artifact_ignores_a_user_literal_containing_the_metadata_marker() {
    let dir = std::env::temp_dir().join(format!("thaw-artifact-section-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(&source, "function main(): void { console.log('THAW_ARTIFACT_V1:{\\\"wrong\\\":true}'); }\n").unwrap();
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let manifest = artifact_manifest_from_bytes(&bytes).unwrap();
    assert_eq!(manifest["quickjs"], false);
    assert!(manifest.get("wrong").is_none());
    run_inspect(&[output.display().to_string()]).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
