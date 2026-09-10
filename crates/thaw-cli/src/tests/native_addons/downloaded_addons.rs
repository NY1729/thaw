#[test]
fn registry_runs_utf8_validate_prebuild_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-utf8-validate-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("utf-8-validate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function isValidUTF8(buffer: any): boolean;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[240,144,128,128]}]"))));
                console.log(Boolean(isValidUTF8(JSON.parse("[{\"type\":\"Buffer\",\"data\":[255]}]"))));
            }
            "#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["utf-8-validate".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_runs_bcrypt_async_callbacks_when_supplied() {
    let Ok(prebuild) = std::env::var("THAW_BCRYPT_NODE") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("thaw-cli-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("bcrypt");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function gen_salt(argsArray: Json): Json;\n\
             declare function encrypt(argsArray: Json): Json;\n\
             declare function compare(argsArray: Json): Json;\n",
    )
    .unwrap();
    std::fs::copy(prebuild, package.join("native.node")).unwrap();
    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.lines().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(stdout.lines().any(|line| line == "false"), "{stdout}");
    assert!(
        stdout.lines().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    assert_eq!(stdout.lines().count(), 4, "{stdout}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_bcrypt_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-bcrypt-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "bcrypt@6.0.0").unwrap();
    let native = added.native_addon.expect("a matching prebuild is bundled");
    assert_eq!(native.platform, "linux");
    assert_eq!(native.arch, "x64");
    assert_eq!(native.libc, "glibc");
    assert!(registry.join("bcrypt/native.node").is_file());
    assert!(registry.join("bcrypt/native-addon.json").is_file());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
            &source,
            r#"function main(): void {
                const salt: Json = callNativeAddon(
                    "gen_salt_sync",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]")
                );
                console.log(String(salt));
                const hash: Json = callNativeAddon(
                    "encrypt_sync",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                );
                console.log(String(hash));
                console.log(Boolean(callNativeAddon(
                    "compare_sync",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]")
                )));
                const saltQueued = callNativeAddonWithCallback(
                    "gen_salt",
                    JSON.parse("[\"b\",4,{\"type\":\"Buffer\",\"data\":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const encryptQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(result));
                        return result;
                    }
                );
                const compareQueued = callNativeAddonWithCallback(
                    "compare",
                    JSON.parse("[\"wrong-password\",\"$2b$04$abcdefghijklmnopqrstuu\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(Boolean(result));
                        return result;
                    }
                );
                const invalidQueued = callNativeAddonWithCallback(
                    "encrypt",
                    JSON.parse("[\"password\",\"invalid\"]"),
                    (error: Json, result: Json): Json => {
                        console.log(String(error));
                        return error;
                    }
                );
            }"#,
        )
        .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["bcrypt".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7, "{stdout}");
    assert!(lines[0].starts_with("$2b$04$"), "{stdout}");
    assert!(lines[1].starts_with("$2b$04$"), "{stdout}");
    assert_eq!(lines[2], "false", "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.starts_with("$2b$04$")),
        "{stdout}"
    );
    assert!(lines[3..].contains(&"false"), "{stdout}");
    assert!(
        lines[3..].iter().any(|line| line.contains("Invalid salt")),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_argon2_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-auto-argon2-{}", std::process::id()));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "argon2@0.44.0").unwrap();
    assert!(added.native_addon.is_some());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        r#"import { hash, verify } from "argon2";
async function main(): Promise<void> {
    const encoded = await hash("password");
    console.log(await verify(encoded, "password"));
    console.log(await verify(encoded, "wrong"));
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["argon2".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\nfalse\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_lists_serial_ports_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-serialport-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "@serialport/bindings-cpp@12.0.1").unwrap();
    assert!(added.native_addon.is_some());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        r#"import { autoDetect } from "@serialport/bindings-cpp";
async function main(): Promise<void> {
    const ports = await autoDetect().list();
    console.log(Array.isArray(ports));
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["@serialport/bindings-cpp".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_fetches_and_runs_node_rs_crc32_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-auto-node-rs-crc32-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let added = thaw_registry::add(&registry, "@node-rs/crc32@1.10.7").unwrap();
    assert!(added.native_addon.is_some());

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        r#"import { crc32, crc32c } from "@node-rs/crc32";
function main(): void {
    console.log(crc32("hello"));
    console.log(crc32c("hello"));
}"#,
    )
    .unwrap();
    build(
        &source,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["@node-rs/crc32".into()],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "907060870\n2591144780\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_builds_a_multi_package_project_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-multi-package-project-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    for package in [
        "zod@3.23.0",
        "nanoid@5.1.5",
        "lodash@4.17.21",
        "@node-rs/crc32@1.10.7",
    ] {
        thaw_registry::add(&registry, package).unwrap();
    }

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        r#"import { z } from "zod";
import { nanoid } from "nanoid";
import { map, reduce } from "lodash";
import { crc32 } from "@node-rs/crc32";
function main(): void {
    const input = z.object({ values: z.array(z.number()) }).parse({ values: [1, 2, 3] });
    const doubled: number[] = map(input.values, (value: number): number => value * 2);
    console.log(reduce(doubled, (sum: number, value: number): number => sum + value, 0));
    const id: string = nanoid(8);
    console.log(id.length);
    console.log(crc32("hello"));
}"#,
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "12\n8\n907060870\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_ws_echo_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-real-ws-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "ws@8.18.3").unwrap();

    let source = dir.join("main.ts");
    let output = dir.join("app");
    std::fs::write(
        &source,
        r#"import { WebSocketServer } from "ws";
async function main(): Promise<void> {
    await new Promise<void>((resolve, reject): void => {
        const server = new WebSocketServer({ port: __PORT__ });
        server.on("error", reject);
        server.on("connection", (socket): void => {
            socket.on("message", (data): void => {
                socket.send(data);
                console.log(data.toString());
                socket.terminate();
                server.close(() => resolve());
            });
        });
    });
}"#
        .replace(
            "__PORT__",
            &(20_000 + std::process::id() % 20_000).to_string(),
        ),
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let child = Command::new(&output)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::{Read, Write};
    let address = format!("127.0.0.1:{}", 20_000 + std::process::id() % 20_000);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut stream = loop {
        match std::net::TcpStream::connect(&address) {
            Ok(stream) => break stream,
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("failed to connect to ws server: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(
            format!(
                "GET / HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut response = [0_u8; 1024];
    let response_len = stream.read(&mut response).unwrap();
    assert!(
        String::from_utf8_lossy(&response[..response_len]).starts_with("HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&response[..response_len])
    );
    stream
        .write_all(&[0x81, 0x85, 1, 2, 3, 4, b'h' ^ 1, b'e' ^ 2, b'l' ^ 3, b'l' ^ 4, b'o' ^ 1])
        .unwrap();
    let mut frame = [0_u8; 7];
    stream.read_exact(&mut frame).unwrap();
    assert_eq!(&frame, b"\x82\x05hello");
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hello\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_a_real_socket_io_echo_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-real-socket-io-{}", std::process::id()));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "socket.io@4.8.1").unwrap();

    let source = dir.join("main.ts");
    let output = dir.join("app");
    let port = 20_000 + std::process::id() % 20_000;
    std::fs::write(
        &source,
r#"import { Server } from "socket.io";
function main(): void {
    const server = new Server(__PORT__, { transports: ["websocket"] });
    server.on("connection", (socket: JsValue): void => {
        socket.on("echo", (value: string): void => {
            socket.emit("echo", value);
        });
    });
}"#
        .replace("__PORT__", &port.to_string()),
    )
    .unwrap();
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let mut child = Command::new(&output).spawn().unwrap();

    use std::io::{Read, Write};
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut stream = loop {
        match std::net::TcpStream::connect(&address) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("failed to connect to Socket.IO server: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).unwrap();
        response.push(byte[0]);
    }
    assert!(
        String::from_utf8_lossy(&response).starts_with("HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&response)
    );

    let write_frame = |stream: &mut std::net::TcpStream, payload: &[u8]| {
        let mut frame = vec![0x81, 0x80 | payload.len() as u8, 1, 2, 3, 4];
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ [1, 2, 3, 4][index % 4]),
        );
        stream.write_all(&frame).unwrap();
    };
    let read_frame = |stream: &mut std::net::TcpStream| {
        let mut header = [0_u8; 2];
        stream.read_exact(&mut header).unwrap();
        let mut payload = vec![0_u8; usize::from(header[1] & 0x7f)];
        stream.read_exact(&mut payload).unwrap();
        payload
    };
    assert!(read_frame(&mut stream).starts_with(b"0{"));
    write_frame(&mut stream, b"40");
    assert!(read_frame(&mut stream).starts_with(b"40"));
    write_frame(&mut stream, br#"42["echo","hello"]"#);
    assert_eq!(read_frame(&mut stream), br#"42["echo","hello"]"#);
    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_add_runs_real_socket_io_client_ack_and_disconnect_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-real-socket-io-client-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    thaw_registry::add(&registry, "socket.io@4.8.1").unwrap();
    thaw_registry::add(&registry, "socket.io-client@4.8.1").unwrap();
    let port = (20_000 + std::process::id() % 20_000) as u16;
    let server_source = dir.join("server.ts");
    let client_source = dir.join("client.ts");
    let server_output = dir.join("server");
    let client_output = dir.join("client");
    std::fs::write(
        &server_source,
        r#"import { Server } from "socket.io";
function main(): void {
    const server = new Server(__PORT__, { transports: ["websocket"] });
    server.on("connection", (socket: JsValue): void => {
        socket.on("echo", (value: string, acknowledge: JsValue): void => {
            acknowledge.call(undefined, "ack:" + value);
            socket.emit("echo", value);
        });
    });
}"#
        .replace("__PORT__", &port.to_string()),
    )
    .unwrap();
    std::fs::write(
        &client_source,
        r#"import { io } from "socket.io-client";
function main(): void {
    const socket = io("http://127.0.0.1:__PORT__", { transports: ["websocket"] });
    socket.on("connect", (): void => {
        socket.emit("echo", "hello", (value: string): void => console.error(value));
    });
    socket.on("echo", (value: string): void => {
        console.error(value);
        socket.disconnect();
    });
    socket.on("disconnect", (): void => console.error("disconnected"));
    socket.on("connect_error", (error: JsValue): void => console.error(error.message));
}"#
        .replace("__PORT__", &port.to_string()),
    )
    .unwrap();
    build(
        &server_source,
        &server_output,
        &[],
        &[],
        &[],
        &registry,
        &[],
    )
    .unwrap();
    build(
        &client_source,
        &client_output,
        &[],
        &[],
        &[],
        &registry,
        &[],
    )
    .unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let mut server = Command::new(&server_output).spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "compiled Socket.IO server did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut client = Command::new(&client_output)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(3));
    if client.try_wait().unwrap().is_none() {
        client.kill().unwrap();
    }
    let client = client.wait_with_output().unwrap();
    server.kill().unwrap();
    server.wait().unwrap();
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(stderr.lines().any(|line| line == "ack:hello"), "{stderr}");
    assert!(stderr.lines().any(|line| line == "hello"), "{stderr}");
    assert!(
        stderr.lines().any(|line| line == "disconnected"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

