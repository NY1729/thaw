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
