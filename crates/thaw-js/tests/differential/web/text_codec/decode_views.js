const d = new TextDecoder();
const buf = new Uint8Array([65, 66, 67, 68]).buffer;
console.log(d.decode(buf));
console.log(d.decode(new Uint8Array(buf, 1, 2)));
console.log(d.decode(new DataView(buf, 2, 2)));
console.log(d.decode(new Uint16Array([0x4241, 0x4443])));
try { d.decode('str'); } catch (e) { console.log(e instanceof TypeError); }
try { d.decode({}); } catch (e) { console.log(e instanceof TypeError); }
try { d.decode(null); } catch (e) { console.log(e instanceof TypeError); }
