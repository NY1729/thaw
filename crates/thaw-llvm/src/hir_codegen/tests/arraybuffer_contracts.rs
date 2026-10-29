// Native `new ArrayBuffer(n)` / `new DataView(..)` / `.buffer`: expected output from node 22.

#[test]
fn arraybuffer_dataview_native_matches_node() {
    let source = r#"
function main(): void {
  const ab = new ArrayBuffer(8);
  console.log(ab.byteLength);
  console.log(ab);
  const dv = new DataView(ab, 0, 8);
  dv.setInt8(0, -2);
  dv.setUint8(1, 255);
  dv.setInt16(2, -1234, true);
  dv.setUint16(2, 65000);
  dv.setInt32(4, -123456789, true);
  console.log(dv.getInt8(0), dv.getUint8(0), dv.getUint8(1), dv.getInt16(2), dv.getInt16(2, true), dv.getUint16(2, true));
  console.log(dv.getInt32(4, true), dv.getUint32(4, true), dv.getInt32(4));
  dv.setFloat32(0, 1.5);
  console.log(dv.getFloat32(0), dv.getFloat32(0, true));
  dv.setFloat64(0, Math.PI, true);
  console.log(dv.getFloat64(0, true), dv.getFloat64(0));
  dv.setBigInt64(0, -5n, true);
  console.log(dv.getBigInt64(0, true), dv.getBigInt64(0));
  console.log(dv.byteLength, dv.byteOffset, dv.buffer.byteLength);
  console.log(dv);
  const u = new Uint8Array(ab);
  u[0] = 7;
  console.log(dv.getUint8(0), u.length);
  dv.setUint8(3, 99);
  console.log(u[3]);
  const b = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]);
  const dv2 = new DataView(b.buffer, b.byteOffset, 8);
  console.log(dv2.getUint32(0), dv2.getUint16(6, true));
  dv2.setUint8(0, 200);
  console.log(b[0]);
  console.log(ab instanceof ArrayBuffer, dv instanceof DataView, ab instanceof DataView, u instanceof ArrayBuffer);
  console.log(ArrayBuffer.isView(dv), ArrayBuffer.isView(u), ArrayBuffer.isView(ab));
  const s = ab.slice(2, 6);
  console.log(s.byteLength, s);
  try { dv.getInt32(6); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { dv.setInt8(-1, 1); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { new DataView(ab, 9); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { new DataView(ab, 0, 20); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { new ArrayBuffer(-1); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  console.log(new ArrayBuffer(0), new ArrayBuffer(60));
  console.log(new DataView(new ArrayBuffer(4), 1, 2));
}
"#;
    let output = compile_and_run(source, "arraybuffer_dataview_native_matches_node");
    assert_eq!(output, "8\nArrayBuffer {\n  [Uint8Contents]: <00 00 00 00 00 00 00 00>,\n  [byteLength]: 8\n}\n-2 254 255 -536 -5891 59645\n-123456789 4171510507 -349002504\n1.5 6.896490392174587e-41\n3.141592653589793 3.207375630676366e-192\n-5n -288230376151711745n\n8 0 8\nDataView {\n  [byteLength]: 8,\n  [byteOffset]: 0,\n  [buffer]: ArrayBuffer {\n    [Uint8Contents]: <fb ff ff ff ff ff ff ff>,\n    [byteLength]: 8\n  }\n}\n7 8\n99\n16909060 2055\n200\ntrue true false false\ntrue true false\n4 ArrayBuffer { [Uint8Contents]: <ff 63 ff ff>, [byteLength]: 4 }\nRangeError Offset is outside the bounds of the DataView\nRangeError Offset is outside the bounds of the DataView\nRangeError Start offset 9 is outside the bounds of the buffer\nRangeError Invalid DataView length 20\nRangeError Invalid array buffer length\nArrayBuffer { [Uint8Contents]: <>, [byteLength]: 0 } ArrayBuffer {\n  [Uint8Contents]: <00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00>,\n  [byteLength]: 60\n}\nDataView {\n  [byteLength]: 2,\n  [byteOffset]: 1,\n  [buffer]: ArrayBuffer { [Uint8Contents]: <00 00 00 00>, [byteLength]: 4 }\n}\n");
}

#[test]
fn arraybuffer_transfer_detached_and_annotations_match_node() {
    let source = r#"
function main(): void {
  const ab = new ArrayBuffer(8);
  console.log(ab.byteLength, ab.detached, ab.resizable, ab.maxByteLength);
  const moved = ab.transfer(16);
  console.log(moved.byteLength, ab.byteLength, ab.detached, moved.detached);
  const fixed = moved.transferToFixedLength(4);
  console.log(fixed.byteLength, moved.detached);
  const keep = new ArrayBuffer(3);
  new Uint8Array(keep)[1] = 7;
  const copy = keep.transfer();
  console.log(copy.byteLength, new Uint8Array(copy)[1], keep.byteLength);
  const ann: ArrayBuffer = new ArrayBuffer(4);
  const u = new Uint8Array(ann);
  u[0] = 5;
  const d2: DataView = new DataView(ann);
  console.log(d2.getUint8(0), ann.byteLength);
  const sliced: ArrayBuffer = ann.slice(1);
  console.log(sliced.byteLength);
  const n = new Uint8Array(ann);
  console.log(n.length, n[0]);
}
"#;
    let output = compile_and_run(source, "arraybuffer_transfer_detached_and_annotations_match_node");
    assert_eq!(output, "8 false false 8\n16 0 true false\n4 true\n3 7 0\n5 4\n3\n4 5\n");
}
