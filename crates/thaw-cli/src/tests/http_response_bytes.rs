fn run_http_bytes_program(name: &str, source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    let _ = std::fs::remove_dir_all(dir);
    (
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
        result.status.code(),
    )
}

/// `res.end(b)` with an annotated `Uint8Array` local under `(req: any, res: any)` must send the bytes
/// verbatim: `c8 01 ff` arrives as `c801ff`, not as the UTF-8-lossy `efbfbd01efbfbd` of a string write.
#[test]
fn node_http_bytes_chunks_of_every_shape_reach_the_wire() {
    let (stdout, stderr, code) = run_http_bytes_program(
        "http-response-bytes",
        r#"import * as http from 'node:http';
function main(): void {
  const srv = http.createServer((req: any, res: any) => {
    const b: Uint8Array = new Uint8Array([200, 1, 255]);
    res.end(b);
  });
  srv.listen(0, '127.0.0.1', () => {
    http.get({ port: (srv.address() as any).port, host: '127.0.0.1' }, (r: any) => {
      const parts: any[] = [];
      r.on('data', (c: any) => parts.push(c));
      r.on('end', () => { console.log(Buffer.concat(parts).toString('hex')); srv.close(); });
    });
  });
}
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "c801ff\n");
}
