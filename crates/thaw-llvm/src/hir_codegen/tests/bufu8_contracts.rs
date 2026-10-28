// Real shared u8 Buffer/Uint8Array views over an ArrayBuffer-like store. Expected output from node 22.

#[test]
fn bufu8_sharing_apis_match_node() {
    let source = r#"
function main(): void {
  const b = Buffer.alloc(8);
  b.writeUInt32BE(0x01020304, 0);
  console.log(b.buffer.byteLength, b.byteOffset, b.byteLength, b.length);
  const s = b.subarray(2, 6);
  console.log(s.byteOffset, s.byteLength, s.buffer.byteLength);
  const u = new Uint8Array(b.buffer, 4, 3);
  u[0] = 200;
  console.log(b[4], u.length, u.byteOffset, u.byteLength);
  b[5] = 77;
  console.log(u[1]);
  const f = Buffer.from(b.buffer, 2, 4);
  f[0] = 9;
  console.log(b.toString("hex"), f.length, f.byteOffset);
  const ab = b.buffer;
  const copy = ab.slice(1, 5);
  copy[0] = 255;
  console.log(b[1], copy.byteLength);
  const dup = Buffer.from(b);
  dup[0] = 66;
  console.log(b[0], dup[0], dup.byteOffset >= 0);
  const u8 = Uint8Array.from([1, 2, 3, 4]);
  const u8s = u8.slice(1, 3);
  u8s[0] = 99;
  const u8v = u8.subarray(1, 3);
  u8v[1] = 55;
  console.log(u8[1], u8[2], u8s[0], u8s.length);
  const bs = b.slice(0, 2);
  bs[0] = 11;
  console.log(b[0]);
}
"#;
    let output = compile_and_run(source, "bufu8_sharing_apis_match_node");
    assert_eq!(output, "8 0 8 8\n2 4 8\n200 3 4 3\n77\n01020904c84d0000 4 2\n2 4\n1 66 true\n2 55 99 2\n11\n");
}

#[test]
fn bufu8_element_semantics_and_generic_array_methods_on_views_match_node() {
    let source = r#"
function sumOf(v: Buffer): number {
  let s = 0;
  for (let i = 0; i < v.length; i++) s += v[i];
  return s;
}
function main(): void {
  const b = Buffer.alloc(8);
  b[0] = 300;
  b[1] = -1;
  b[2] = 1.9;
  b[3] = NaN;
  b[4] = 256;
  b[5] = Infinity;
  b[6] = -0.5;
  console.log(b.toString("hex"), b[8], b[-1], b[1.5]);
  const v = b.subarray(1, 6);
  console.log([...v], v.map((x) => x * 2).join(","), v.filter((x) => x > 0).length, v.reduce((a, x) => a + x, 0));
  let it = 0;
  for (const x of v) it += x;
  console.log(it, sumOf(v), sumOf(b.subarray(2)), JSON.stringify(v), v.includes(255), v.indexOf(1), v.join("-"));
  const over = Buffer.from([1, 2, 3, 4, 5, 6]);
  over.copy(over, 2, 0, 4);
  console.log(over.toString("hex"));
  const sh = Buffer.from([1, 2, 3, 4, 5, 6]);
  sh.subarray(0, 4).copy(sh.subarray(2));
  console.log(sh.toString("hex"));
  const rs = Buffer.from([9, 8, 7, 6, 5]);
  const rv = rs.subarray(1, 4);
  rv.reverse();
  rv.sort();
  console.log(rs.toString("hex"), rv.fill(1, 1).toString("hex"), rs.toString("hex"));
  const a1 = Buffer.from("abc");
  const a2 = Buffer.from("xabcx").subarray(1, 4);
  console.log(a1.equals(a2), Buffer.concat([a1, a2.subarray(1)]).toString(), a2.toString(), Buffer.isBuffer(a2));
  console.log(a2, [a2, a1], { v: a2 });
  const big = new Uint8Array(0);
  console.log(big.length);
}
"#;
    let output = compile_and_run(source, "bufu8_element_semantics_generic_ops");
    assert_eq!(output, "2cff010000000000 undefined undefined undefined\n[ 255, 1, 0, 0, 0 ] 254,2,0,0,0 2 256\n256 256 1 {\"type\":\"Buffer\",\"data\":[255,1,0,0,0]} true 1 255-1-0-0-0\n010201020304\n010201020304\n0906070805 060101 0906010105\ntrue abcbc abc true\n<Buffer 61 62 63> [ <Buffer 61 62 63>, <Buffer 61 62 63> ] { v: <Buffer 61 62 63> }\n0\n");
}
