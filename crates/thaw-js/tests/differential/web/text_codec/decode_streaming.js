const d = new TextDecoder();
const bytes = new TextEncoder().encode('a😀b');
let out = [];
for (const b of bytes) out.push(d.decode(Uint8Array.of(b), { stream: true }));
out.push(d.decode());
console.log(JSON.stringify(out));
const s = new TextDecoder('utf-8', { fatal: true });
s.decode(Uint8Array.from([0xf0, 0x9f]), { stream: true });
try { s.decode(); console.log('no throw'); } catch (e) { console.log(e instanceof TypeError); }
const bom = new TextDecoder();
console.log(bom.decode(Uint8Array.from([0xef, 0xbb]), { stream: true }).length, bom.decode(Uint8Array.from([0xbf, 0x41])).length);
