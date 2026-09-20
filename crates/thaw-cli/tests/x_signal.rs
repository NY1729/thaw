#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn x_receives_signals_as_the_executed_cli() {
    let root = std::env::temp_dir().join(format!("thaw-x-signal-{}", std::process::id()));
    let package = root.join("node_modules/signal-cli");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"signal-cli","version":"1.0.0","bin":"cli.sh"}"#,
    )
    .unwrap();
    let started = root.join("started");
    let received = root.join("received");
    let pid_file = root.join("pid");
    let bin = package.join("cli.sh");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho $$ > {}\necho started > {}\ntrap 'echo term > {}; exit 0' TERM\nwhile :; do sleep 1; done\n",
            pid_file.display(),
            started.display(),
            received.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let status = Command::new("sh")
        .arg("-c")
        .arg(
            r#""$THAW_BIN" x --no-install signal-cli &
pid=$!
while test ! -f "$STARTED"; do sleep 0.01; done
kill -TERM "$pid"
wait "$pid""#,
        )
        .env("THAW_BIN", env!("CARGO_BIN_EXE_thaw"))
        .env("STARTED", &started)
        .current_dir(&root)
        .status()
        .unwrap();

    if !status.success() {
        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            let _ = Command::new("kill").args(["-KILL", pid.trim()]).status();
        }
    }
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(received).unwrap(), "term\n");
    let _ = std::fs::remove_dir_all(root);
}
