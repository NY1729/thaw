// Buffer/fs/crypto/TextDecoder/process behaviour fixed against Node 22 (end-to-end builds).

#[test]
fn node_fs_writes_buffer_bytes_and_read_sync_fills_the_buffer() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node_fs_writes_buffer_bytes_and_read_sync_fills_the_buffer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"import * as fs from "node:fs";
function main(): void {
  const d = "DIR";
  fs.mkdirSync(d, { recursive: true });
  fs.writeFileSync(d + "/a", Buffer.from([1, 2, 3]));
  console.log(fs.readFileSync(d + "/a"));
  fs.writeFileSync(d + "/b", Buffer.from([65, 200, 3, 0, 255]));
  console.log(fs.statSync(d + "/b").size, fs.readFileSync(d + "/b"));
  fs.appendFileSync(d + "/b", Buffer.from([7, 8]));
  console.log(fs.readFileSync(d + "/b"));
  fs.writeFileSync(d + "/c", "hello world");
  const fd = fs.openSync(d + "/c", "r");
  const buf = Buffer.alloc(8);
  const n = fs.readSync(fd, buf, 2, 5, 6);
  console.log(n, buf);
  const first = Buffer.alloc(4);
  const m = fs.readSync(fd, first, 0, 4, 0);
  console.log(m, first.toString());
  fs.closeSync(fd);
  fs.rmSync(d, { recursive: true });
}
"#.replace("DIR", &dir.join("data").display().to_string());
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Expected output produced by Node 22 running the same program.
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "<Buffer 01 02 03>\n5 <Buffer 41 c8 03 00 ff>\n<Buffer 41 c8 03 00 ff 07 08>\n5 <Buffer 00 00 77 6f 72 6c 64 00>\n4 hell\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_crypto_md5_sha3_hash_and_subtle_match_node() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node_crypto_md5_sha3_hash_and_subtle_match_node-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"import * as crypto from "node:crypto";
import { createHash } from "node:crypto";
function main(): void {
  console.log(createHash("md5").update("a").digest("hex"));
  console.log(crypto.createHash("sha3-256").update("x").digest("hex"));
  console.log(crypto.createHash("sha3-512").update("abc").digest("base64"));
  console.log(crypto.createHmac("md5", Buffer.from("k")).update("a").update("b").digest("base64"));
  console.log(crypto.createHmac("sha3-256", "key").update("hello").digest("hex"));
  console.log(crypto.hash("sha1", "abc"));
  console.log(crypto.hash("sha3-224", "abc", "base64"));
  console.log(typeof crypto.webcrypto.subtle, typeof crypto.subtle);
}
"#;
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Expected output produced by Node 22 running the same program.
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "0cc175b9c0f1b6a831c399e269772661\n741efa311f97686956946758e0d95f70f11ff2da4f2feb7c54314f44134ac49f\nt1GFCxpXFopWk82SS2sJbgj2IYJ0RPcNiE9dAkDScS4Q4RbpGSrzyRp+xXZH45NAVzQLTPQI1aVlkvgnTuxT8A==\nPAkBUEetz4Opb1/fp1/eWQ==\na59b86b015726e63cb275f18729321e3b59068796945c7c99ddb7438b7539854\na9993e364706816aba3e25717850c26c9cd0d89d\n5kKCTD+M8krQkjTufTx2b8mjpRaNDJStc7Rv3w==\nobject object\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn text_decoder_decodes_utf16le_and_latin1() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-text_decoder_decodes_utf16le_and_latin1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"function main(): void {
  console.log(new TextDecoder("utf-16le").decode(new Uint8Array([104, 0, 105, 0])));
  console.log(new TextDecoder("utf-16le").decode(new Uint8Array([0x3d, 0xd8, 0x00, 0xde, 0x41, 0])));
  console.log(new TextDecoder("latin1").decode(new Uint8Array([233, 65, 0x80])));
  console.log(new TextDecoder("latin1").encoding, new TextDecoder("utf-16").encoding);
}
"#;
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Expected output produced by Node 22 running the same program.
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "hi\n😀A\néA€\nwindows-1252 utf-16le\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn process_hrtime_bigint_ids_release_and_self_kill_match_node() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-process_hrtime_bigint_ids_release_and_self_kill_match_node-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"function main(): void {
  const b = process.hrtime.bigint();
  console.log(typeof b, b > 0n, process.hrtime.bigint() >= b);
  console.log(typeof process.uptime(), typeof process.memoryUsage.rss(), typeof process.getuid(), typeof process.getgid(), typeof process.umask());
  console.log(process.release.name, process.kill(process.pid, 0));
  process.on("SIGUSR2", () => { console.log("got sig"); });
  process.kill(process.pid, "SIGUSR2");
  setTimeout(() => console.log("end"), 20);
}
"#;
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Expected output produced by Node 22 running the same program.
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "bigint true true\nnumber number number number number\nnode true\ngot sig\nend\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
