const d = new TextDecoder();
console.log(d.encoding, d.fatal, d.ignoreBOM);
console.log(d.decode(new TextEncoder().encode('héllo😀')));
console.log(d.decode(Uint8Array.from([0x61, 0xff, 0x62])));
console.log(d.decode());
console.log(d.decode(Uint8Array.from([0xef, 0xbb, 0xbf, 0x61])).length);
console.log(new TextDecoder('utf-8', { ignoreBOM: true }).decode(Uint8Array.from([0xef, 0xbb, 0xbf, 0x61])).length);
let fatal = false;
try { new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from([0xc0, 0x80])); } catch (e) { fatal = e instanceof TypeError; }
console.log(fatal);
console.log(new TextDecoder('utf-16le').decode(Uint8Array.from([0x61, 0, 0x3d, 0xd8, 0x00, 0xde])));
console.log(new TextDecoder('latin1').encoding, new TextDecoder('latin1').decode(Uint8Array.from([0xe9, 0x80])));
try { new TextDecoder('nope'); } catch (e) { console.log(e instanceof RangeError); }
