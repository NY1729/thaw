const e = new TextEncoder();
const out = new Uint8Array(5);
console.log(JSON.stringify(e.encodeInto('A😀B', out)), Array.from(out).join());
const big = new Uint8Array(16);
console.log(JSON.stringify(e.encodeInto('héllo', big)), Array.from(big.subarray(0, 6)).join());
console.log(JSON.stringify(e.encodeInto('', new Uint8Array(0))));
try { e.encodeInto('x', new Uint16Array(2)); } catch (err) { console.log(err instanceof TypeError); }
try { e.encodeInto('x', []); } catch (err) { console.log(err instanceof TypeError); }
