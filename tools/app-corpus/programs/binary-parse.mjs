// Binary parsing: a small tagged record format with Buffer, DataView and TextEncoder/TextDecoder.
const enc = new TextEncoder();
const dec = new TextDecoder();

function encodeRecord({ id, name, score, flags }) {
  const nameBytes = enc.encode(name);
  const buf = Buffer.alloc(4 + 2 + nameBytes.length + 8 + 1);
  let o = 0;
  o = buf.writeUInt32BE(id, o);
  o = buf.writeUInt16LE(nameBytes.length, o);
  o += buf.write(name, o, 'utf8');
  o = buf.writeDoubleLE(score, o);
  buf.writeUInt8(flags, o);
  return buf;
}

function decodeRecord(buf) {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const id = dv.getUint32(0, false);
  const len = dv.getUint16(4, true);
  const name = dec.decode(buf.subarray(6, 6 + len));
  const score = dv.getFloat64(6 + len, true);
  const flags = dv.getUint8(14 + len);
  return { id, name, score, flags: flags.toString(2).padStart(8, '0') };
}

const recs = [
  { id: 1, name: 'alpha', score: 3.14159, flags: 0b1010 },
  { id: 4294967295, name: 'héllo wörld ✓', score: -0.5, flags: 255 },
];
const blob = Buffer.concat(recs.map(encodeRecord));
console.log(blob.length, blob.toString('hex').slice(0, 40));
console.log(blob.toString('base64').slice(0, 30));
let off = 0;
while (off < blob.length) {
  const len = blob.readUInt16LE(off + 4);
  const size = 4 + 2 + len + 8 + 1;
  console.log(decodeRecord(blob.subarray(off, off + size)));
  off += size;
}
console.log(Buffer.from('68656c6c6f', 'hex').toString(), Buffer.from('aGk=', 'base64').toString());
console.log(new Uint8Array([1, 2, 3]).map((x) => x * 100), Buffer.compare(Buffer.from('a'), Buffer.from('b')));
console.log(enc.encode('€').length, dec.decode(new Uint8Array([0xf0, 0x9f, 0x98, 0x80])));
