// Shared Buffer views (subarray/slice write-through). Expected output from node 22.

#[test]
fn buffer_views_share_writes_both_ways() {
    let source = r#"
function main(): void {
  const b = Buffer.from([1, 2, 3, 4]);
  const s = b.subarray(1, 3);
  s[0] = 99;
  console.log(b[1], s.length, s.byteOffset - b.byteOffset);
  b[2] = 77;
  console.log(s[1], s.toString("hex"));
  const t = b.slice(2);
  t[0] = 7;
  console.log(b[2], s[1], t.length);
  const h = Buffer.from("hello");
  const e = h.subarray(1, 3);
  h[1] = 69;
  console.log(e.toString());
  const hdr = Buffer.alloc(8);
  const body = hdr.subarray(4);
  body.writeUInt32BE(0xdeadbeef, 0);
  console.log(hdr.toString("hex"));
  hdr.writeUInt16LE(0x1234, 4);
  console.log(body.toString("hex"));
  const nested = hdr.subarray(2).subarray(2);
  nested[0] = 1;
  console.log(hdr.toString("hex"), nested.byteOffset - hdr.byteOffset);
  const f = Buffer.alloc(6);
  const fv = f.subarray(2, 5);
  fv.fill(9);
  console.log(f.toString("hex"));
  f.fill(1);
  console.log(fv.toString("hex"));
  const c = Buffer.from([5, 6, 7, 8, 9]);
  const cv = c.subarray(1, 4);
  Buffer.from([1, 2]).copy(cv, 1);
  console.log(c.toString("hex"));
  cv.reverse();
  console.log(c.toString("hex"));
  cv[0]++;
  cv[1] += 2;
  console.log(c.toString("hex"));
  console.log(b.subarray(-2).length, b.subarray(5).length, b.subarray(1, 100).length);
  const copy = Buffer.from(b);
  copy[0] = 200;
  console.log(b[0]);
}
"#;
    let output = compile_and_run(source, "buffer_views_share_writes_both_ways");
    assert_eq!(output, "99 2 1\n77 634d\n7 7 2\nEl\n00000000deadbeef\n3412beef\n000000000112beef 4\n000009090900\n010101\n0506010209\n0502010609\n0503030609\n2 0 3\n1\n");
}

#[test]
fn buffer_views_survive_fill_copy_sort_and_frames() {
    let source = r#"
function fillView(v: Buffer, n: number): void {
  for (let i = 0; i < v.length; i++) v[i] = n + i;
}
class Frame {
  data: Buffer;
  payload: Buffer;
  constructor(size: number) {
    this.data = Buffer.alloc(size);
    this.payload = this.data.subarray(2);
  }
  setLen(n: number): void { this.data.writeUInt16BE(n, 0); }
  put(i: number, x: number): void { this.payload[i] = x; }
}
function main(): void {
  const big = Buffer.alloc(10);
  fillView(big.subarray(2, 6), 10);
  console.log(big.toString("hex"));
  const f = new Frame(6);
  f.setLen(0x0102);
  f.put(1, 255);
  console.log(f.data.toString("hex"), f.payload.toString("hex"), f.payload.byteOffset - f.data.byteOffset);
  const a = big.subarray(0, 4);
  const b = big.subarray(2, 8);
  a[3] = 200;
  console.log(b[1], b.byteLength, a.equals(big.subarray(0, 4)));
  b.writeFloatLE(1.5, 0);
  console.log(big.readFloatLE(2), a.toString("hex"));
  big.subarray(2, 10).writeBigInt64BE(-2n, 0);
  console.log(big.toString("hex"));
  console.log(Buffer.concat([a, b]).toString("hex"));
  const views: Buffer[] = [];
  for (let i = 0; i < 5; i++) views.push(big.subarray(i, i + 2));
  views[4][1] = 1;
  console.log(big[5], views[3][2 - 1] === big[4]);
  const tmp = Buffer.from("abcdef");
  const part = tmp.subarray(2, 4);
  part.set([120, 121]);
  console.log(tmp.toString());
  const sorted = Buffer.from([5, 3, 9, 1]);
  const sv = sorted.subarray(1, 3);
  sv.sort();
  console.log(sorted.toString("hex"));
  const sl = Buffer.from("hello world");
  const word = sl.slice(6);
  word[0] = 87;
  console.log(sl.toString(), word.toString(), sl.subarray(6, 8).toString());
  let total = 0;
  const chunk = Buffer.alloc(1000);
  for (let i = 0; i < 1000; i++) { const w = chunk.subarray(i, i + 1); w[0] = i & 255; total += w[0]; }
  console.log(total, chunk[999], chunk[300]);
  const copy = Buffer.from(big.subarray(0, 3));
  copy[0] = 123;
  console.log(big[0], copy[0]);
  const z = big.subarray(3, 3);
  console.log(z.length, z.toString("hex") === "");
}
"#;
    let output = compile_and_run(source, "buffer_views_survive_fill_copy_sort_and_frames");
    assert_eq!(output, "00000a0b0c0d00000000\n010200ff0000 00ff0000 2\n200 6 true\n1.5 00000000\n0000fffffffffffffffe\n0000ffffffffffffffff\n1 true\nabxyef\n05030901\nhello World World Wo\n124716 231 44\n0 123\n0 true\n");
}

#[test]
fn buffer_views_many_small_and_large_views() {
    let source = r#"
function main(): void {
  const chunk = Buffer.alloc(100000);
  let total = 0;
  for (let i = 0; i < 100000; i++) { const w = chunk.subarray(i, i + 4 > 100000 ? 100000 : i + 4); w[0] = i & 255; total += w[0]; chunk[(i * 7) % 100000] = 1; }
  console.log(total, chunk[99999], chunk[300]);
  const big = Buffer.alloc(4 * 1024 * 1024);
  const v = big.subarray(10);
  for (let i = 0; i < 200000; i++) { v[i] = i & 255; }
  console.log(big[10 + 4000], big[10]);
}
"#;
    let output = compile_and_run(source, "buffer_views_many_small_and_large_views");
    assert_eq!(output, "12742320 159 1\n160 0\n");
}
