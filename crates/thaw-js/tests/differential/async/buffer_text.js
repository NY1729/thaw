const b = Buffer.from('héllo wörld €😀');
console.log(b, b.length, b.toString(), b.toString('hex'), b.toString('base64'), b.toString('base64url'), b.toString('latin1').length, b.toString('utf8', 1, 4));
console.log(Buffer.from('68656c6c6f', 'hex').toString(), Buffer.from('aGVsbG8=', 'base64').toString(), Buffer.from('aGVsbG8', 'base64').toString(), Buffer.from('_-8', 'base64').join(), Buffer.from('zz', 'hex').length);
console.log(Buffer.from([1, 2, 3, 256, -1]), Buffer.alloc(3), Buffer.alloc(4, 'ab'), Buffer.alloc(3, 7), Buffer.from(new Uint8Array([9, 8]).buffer), Buffer.from(Buffer.from('xy')));
console.log(Buffer.concat([Buffer.from('ab'), Buffer.from('cd')]).toString(), Buffer.concat([Buffer.from('abc')], 2).toString(), Buffer.byteLength('€'), Buffer.byteLength('abc', 'hex') , Buffer.isBuffer(b), Buffer.isBuffer(new Uint8Array(1)), b instanceof Uint8Array);
console.log(Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from('a').equals(Buffer.from('a')), Buffer.from('abc').compare(Buffer.from('abd')));
const w = Buffer.alloc(16);
w.writeUInt8(255, 0); w.writeUInt16BE(0x1234, 1); w.writeUInt32LE(0xdeadbeef, 3); w.writeInt16LE(-2, 7); w.writeFloatBE(1.5, 9); w.writeDoubleLE(Math.PI, 8);
console.log(w, w.readUInt8(0), w.readUInt16BE(1), w.readUInt32LE(3), w.readInt16LE(7), w.readFloatBE(9).toFixed(3), w.readDoubleLE(8), w.readInt8(0), w.readUint8(0));
console.log(w.slice(1, 3), w.subarray(3, 5).length, Buffer.from('hello').slice(1, 3).toString(), Buffer.from('hello').indexOf('l'), Buffer.from('hello').lastIndexOf('l'), Buffer.from('hello').includes('ell'), Buffer.from('hello').indexOf(Buffer.from('lo')), Buffer.from('hello').indexOf(111));
const tgt = Buffer.alloc(5, '.'); Buffer.from('xyz').copy(tgt, 1); console.log(tgt.toString(), Buffer.from('abc').fill('z').toString(), tgt.write('QQ', 3), tgt.toString());
console.log(JSON.stringify(Buffer.from('hi')), Buffer.from('hi').toJSON(), Buffer.from(JSON.parse(JSON.stringify(Buffer.from('hi')))).toString());
const enc = new TextEncoder(), dec = new TextDecoder();
const e = enc.encode('€uro 😀'); console.log(e.length, e.constructor.name, dec.decode(e), dec.encoding, enc.encoding, new TextDecoder('utf-8', { fatal: false }).decode(new Uint8Array([0xff, 0x41])));
console.log(new TextDecoder().decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x68, 0x69])), new TextDecoder('latin1').decode(new Uint8Array([0xe9])), new TextDecoder('utf-16le').decode(new Uint8Array([0x68, 0, 0x69, 0])));
try { new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff])); } catch (er) { console.log(er.name, er.code); }
try { Buffer.alloc(2).readUInt32LE(0); } catch (er) { console.log(er.name, er.code); }
try { Buffer.from(5); } catch (er) { console.log(er.name, er.code); }
try { Buffer.from('a', 'nope'); } catch (er) { console.log(er.name, er.code, er.message); }
console.log(new TextEncoder().encodeInto('héy', new Uint8Array(3)), atob(btoa('hi')), Buffer.from('hi').toString('ascii'), Buffer.from('€').toString('latin1'), Buffer.from('hi', 'utf16le'), Buffer.from('hi', 'ucs2').toString('utf16le'));
