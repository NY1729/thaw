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

/// fs APIs given a file: `URL` object (AOT `new URL`, `import.meta.url` derived) must resolve it like
/// Node's fileURLToPath; the URL used to be stringified into the path (a stray `file:` directory).
#[test]
fn node_fs_accepts_file_url_objects_from_compiled_code() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node_fs_accepts_file_url_objects-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.txt"), "A\n").unwrap();
    std::fs::write(dir.join("index.html"), "<p>\n").unwrap();
    let entry = dir.join("main.mjs");
    std::fs::write(&entry, r#"import { cp, mkdir, rm, copyFile, readFile, writeFile, stat, readdir, rename, unlink, access } from 'node:fs/promises';
import * as fs from 'node:fs';
const root = new URL('./', import.meta.url);
const out = new URL('dist/', root);
await rm(out, { recursive: true, force: true });
await mkdir(new URL('vendor/', out), { recursive: true });
await cp(new URL('src/', root), new URL('src/', out), { recursive: true });
await copyFile(new URL('index.html', root), new URL('index.html', out));
await writeFile(new URL('w.txt', out), 'hi');
console.log(await readFile(new URL('w.txt', out), 'utf8'));
await rename(new URL('w.txt', out), new URL('v.txt', out));
console.log((await stat(new URL('v.txt', out))).size);
console.log((await readdir(out)).sort().join(','));
await unlink(new URL('v.txt', out));
await access(new URL('index.html', out));
fs.mkdirSync(new URL('sync/', out));
fs.writeFileSync(new URL('sync/a', out), 's');
fs.copyFileSync(new URL('sync/a', out), new URL('sync/b', out));
fs.cpSync(new URL('src/', root), new URL('sync/src/', out), { recursive: true });
console.log(fs.readFileSync(new URL('sync/b', out), 'utf8'), fs.existsSync(new URL('sync/a', out)), fs.statSync(new URL('sync/a', out)).size);
console.log(fs.readdirSync(new URL('sync/', out)).sort().join(','));
fs.renameSync(new URL('sync/a', out), new URL('sync/c', out));
fs.rmSync(new URL('sync/c', out));
try { await readFile(new URL('http://x/y')); } catch (e) { console.log(e.code); }
console.log(fs.readdirSync('.').filter((n) => n.startsWith('file')).length);
fs.readFile(new URL('sync/b', out), 'utf8', (err, data) => {
  console.log(err, data);
  fs.copyFile(new URL('sync/b', out), new URL('sync/d', out), (err) => {
    console.log(err, fs.existsSync(new URL('sync/d', out)));
    fs.mkdir(new URL('cb/', out), (err) => {
      console.log(err, fs.existsSync(new URL('cb/', out)));
      fs.rm(new URL('cb/', out), { recursive: true }, (err) => {
        console.log(err, fs.existsSync(new URL('cb/', out)));
        fs.rmSync(out, { recursive: true });
      });
    });
  });
});
"#).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).current_dir(&dir).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    // Expected output produced by Node 22 running the same program.
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "hi\n2\nindex.html,src,v.txt,vendor\ns true 1\na,b,src\nERR_INVALID_URL_SCHEME\n0\nnull s\nnull true\nnull true\nnull false\n"
    );
    assert!(!dir.join("file:").exists());
    assert!(!dir.join("dist").exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// Invalid argument types for fs/fs.promises/path/url/util/querystring throw Node's ERR_INVALID_ARG_TYPE (code and
/// message) instead of being stringified. The expected text is Node 22 running tests/input_validation/main.ts.
#[test]
fn node_invalid_argument_types_throw_like_node() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node_invalid_argument_types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, include_str!("input_validation/main.ts")).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).current_dir(&dir).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), include_str!("input_validation/expected.txt"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Awaited FileHandle methods inside nested async arrows (try/catch/finally, rethrow, union returns,
/// `(await x).m()` receivers) settle, reject and segfault-free like Node 22 (tests/fsp_filehandle_async/main.ts).
#[test]
fn fsp_filehandle_async_arrows_match_node() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-fsp_filehandle_async-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, include_str!("fsp_filehandle_async/main.ts")).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).current_dir(&dir).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), include_str!("fsp_filehandle_async/expected.txt"));
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

// A thrown error inside an async host callback no longer aborts the process at exit (a lease held
// in a thread-local dropped after the pin table it needs), `(await readdir()).sort()` as a tail
// `return await` keeps its async frame, and a `Promise<Buffer>` callback result is a Buffer, not
// the byte view read as `f64` slots. Expected output from node 22.
#[test]
fn async_callbacks_nested_await_thrown_error_and_buffer_result() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-async_callbacks_nested_await-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    let source = r#"import * as fsp from "node:fs/promises";
import * as path from "node:path";
const Q: any[] = [];
function show(v: any): string {
  if (Array.isArray(v)) { return "array"; }
  if (Buffer.isBuffer(v)) { return "Buffer " + v.length + " " + v.toString("hex"); }
  return String(v);
}
async function main(): Promise<void> {
  Q.push(async (d: any) => { return path.join(d, "b"); });
  Q.push(async (d: string) => { return await (await fsp.readdir(d)).sort(); });
  Q.push(async (d: string) => { return Buffer.from("hello"); });
  Q.push(async (d: string) => { return Buffer.alloc(3); });
  const join: any = Q[0];
  try { await join(5); } catch (e: any) { console.log(e.code); }
  const readdir: any = Q[1];
  console.log(String(await readdir("/proc/self")).length > 0);
  const hello: any = Q[2];
  const zeros: any = Q[3];
  console.log(show(await hello("x")), show(await zeros("x")));
}
"#;
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "ERR_INVALID_ARG_TYPE\ntrue\nBuffer 5 68656c6c6f Buffer 3 000000\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
