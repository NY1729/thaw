// Audit silent-wrong-result fixes (top-level recursion, Record missing keys, object-literal accessors,
// readSync into a top-level Buffer, top-level Buffer-ness). Expected output recorded from node 22.

#[test]
fn audit_silent_recursive() {
    let source = r##"const fact = function f(n: number): number { return n <= 1 ? 1 : n * f(n - 1); };
const fib = (n: number): number => (n < 2 ? n : fib(n - 1) + fib(n - 2));
console.log(fact(5), fib(15));
"##;
    assert_eq!(compile_and_run(source, "audit_silent_recursive"), "120 610\n");
}

#[test]
fn audit_silent_dict() {
    let source = r##"const counts: Record<string, number> = {};
for (const w of "a b a c b a".split(" ")) counts[w] = (counts[w] ?? 0) + 1;
console.log(counts, Object.entries(counts).sort((x, y) => y[1] - x[1]));
const names: Record<string, string> = {};
console.log(names["x"] ?? "none", names["x"] === undefined, counts["a"] ?? -1, counts["zz"] ?? -1);
"##;
    assert_eq!(compile_and_run(source, "audit_silent_dict"), "{ a: 3, b: 2, c: 1 } [ [ 'a', 3 ], [ 'b', 2 ], [ 'c', 1 ] ]\nnone true 3 -1\n");
}

#[test]
fn audit_silent_getter() {
    let source = r##"const o = { _v: 1, get v() { return this._v; }, set v(x: number) { this._v = x * 2; } };
o.v = 5;
console.log(o.v, o._v);
console.log(JSON.stringify(o));
console.log(Object.keys(o));
"##;
    assert_eq!(compile_and_run(source, "audit_silent_getter"), "10 10\n{\"_v\":10,\"v\":10}\n[ '_v', 'v' ]\n");
}

#[test]
fn audit_silent_readsync() {
    let source = r##"import fs from "node:fs";
const path = "/tmp/thaw-audit-silent-1-readsync.txt";
fs.writeFileSync(path, "hello");
const fd = fs.openSync(path, "r");
const b = Buffer.alloc(3);
const n = fs.readSync(fd, b, 0, 3, 1);
fs.closeSync(fd);
fs.unlinkSync(path);
console.log(n, b.toString(), b);
"##;
    assert_eq!(compile_and_run(source, "audit_silent_readsync"), "3 ell <Buffer 65 6c 6c>\n");
}

#[test]
fn audit_silent_buffer() {
    let source = r##"const b = Buffer.from("héllo wörld");
console.log(b.indexOf("w"), b.includes("wör"), b.lastIndexOf("l"));
const s = Buffer.from("abcdef");
console.log(s.slice(1, 3), s.subarray(2).length, s.at(-1));
console.log(Buffer.alloc(3, "ab"), Buffer.alloc(3, 1));
let acc = Buffer.alloc(0);
for (const c of ["a", "b"]) acc = Buffer.concat([acc, Buffer.from(c)]);
console.log(acc.toString(), acc);
console.log(Buffer.from("abc").map((x) => x + 1).toString());
"##;
    assert_eq!(compile_and_run(source, "audit_silent_buffer"), "7 true 11\n<Buffer 62 63> 4 102\n<Buffer 61 62 61> <Buffer 01 01 01>\nab <Buffer 61 62>\nbcd\n");
}

#[test]
fn audit_silent_uint8array_print() {
    let source = r##"const b = Buffer.from("ab");
console.log(new Uint8Array(b), Uint8Array.from(b), Uint8Array.from([1, 2]));
console.log(b, new Uint8Array([3, 4]));
"##;
    assert_eq!(compile_and_run(source, "audit_silent_uint8array_print"), "Uint8Array(2) [ 97, 98 ] Uint8Array(2) [ 97, 98 ] Uint8Array(2) [ 1, 2 ]\n<Buffer 61 62> Uint8Array(2) [ 3, 4 ]\n");
}

