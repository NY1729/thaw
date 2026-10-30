// Audit2 async/streams rows that import Node builtins. Expected output recorded from node 22
// (--experimental-strip-types).

fn run_async_streams_program(name: &str, source: &str) -> (String, String, Option<i32>) {
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

#[test]
fn async_streams_readable_event_fires_and_writable_honours_high_water_mark() {
    let (stdout, stderr, code) = run_async_streams_program(
        "async-streams-readable",
        r##"import { Readable, Writable } from "node:stream";
function main(): void {
  const r = new Readable({ read() {} });
  r.push("abc"); r.push("def"); r.push(null);
  r.setEncoding("utf8");
  r.on("readable", () => { let c; while ((c = r.read()) !== null) console.log("read", c); });
  r.on("end", () => console.log("end"));
  const log: string[] = [];
  const w = new Writable({ highWaterMark: 3, write(c: any, _e: any, cb: any) { log.push("w:" + String(c)); setTimeout(cb, 1); } });
  console.log(w.write("ab"), w.writableLength, w.writableHighWaterMark);
  console.log(w.write("cd"), w.writableLength, w.writableNeedDrain);
  w.on("drain", () => console.log("drain", w.writableLength));
  w.on("finish", () => console.log("finish", log.join()));
  w.end();
}
"##,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "true 2 3\nfalse 4 true\nread abcdef\nend\nfinish w:ab,w:cd\n");
}

#[test]
fn async_streams_uncaught_async_error_exits_one_while_a_server_is_listening() {
    let (_, stderr, code) = run_async_streams_program(
        "async-streams-server-uncaught",
        r##"import * as http from "node:http";
function main(): void {
  const srv = http.createServer((req, res) => { res.end("x"); });
  srv.listen(0, "127.0.0.1", async () => {
    await new Promise<void>((r) => setTimeout(r, 5));
    throw new Error("boom");
  });
}
"##,
    );
    assert!(stderr.contains("Error: boom"), "stderr: {stderr}");
    assert_eq!(code, Some(1), "stderr: {stderr}");
}

#[test]
fn async_streams_this_in_an_event_emitter_listener_is_the_emitter() {
    let (stdout, stderr, code) = run_async_streams_program(
        "async-streams-emitter-this",
        r##"import { EventEmitter } from "node:events";
function main(): void {
  const e = new EventEmitter();
  e.on("y", function (this: any) { console.log(typeof this, this === e); });
  e.on("z", function (this: any, a: number) { console.log(this === e, a); });
  e.once("w", function (this: EventEmitter) { console.log(this === e); });
  e.emit("y"); e.emit("z", 5); e.emit("w");
}
"##,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "object true\ntrue 5\ntrue\n");
}
