const f = new File(['hello ', new Uint8Array([119, 111, 114, 108, 100])], 'greeting.txt', { type: 'Text/Plain', lastModified: 1700000000000 });
console.log(f.name, f.size, f.type, f.lastModified, typeof f.webkitRelativePath, 'webkitRelativePath' in f);
console.log(f instanceof File, f instanceof Blob, Object.prototype.toString.call(f), String(f));
const before = Date.now();
const d = new File([], 'd.bin');
const after = Date.now();
console.log(typeof d.lastModified, d.lastModified >= before && d.lastModified <= after, JSON.stringify(d.type), d.size, d.name);
console.log(new File(['x'], 42, { lastModified: '5' }).name, new File(['x'], 'n', { lastModified: '5' }).lastModified);
console.log(File.length, File.name, Object.getOwnPropertyDescriptor(File.prototype, 'name').enumerable, Object.getOwnPropertyDescriptor(File.prototype, 'constructor').enumerable);
const desc = Object.getOwnPropertyDescriptor(globalThis, 'File');
console.log(typeof desc.value, desc.writable, desc.enumerable, desc.configurable);
const s = f.slice(0, 5, 'Text/Html');
console.log(s instanceof Blob, s instanceof File, s.size, s.type);
for (const args of [[], [['a']]]) {
  try { new File(...args); } catch (e) { console.log(e.name, e.code, e.message, Object.keys(e).join()); }
}
try { File(['a'], 'x'); } catch (e) { console.log(e.name, e.message); }
(async () => {
  console.log(JSON.stringify(await f.text()));
  console.log((await f.arrayBuffer()).byteLength, await s.text());
  const fd = new FormData();
  fd.append('k', new Blob(['ab'], { type: 'text/plain' }));
  const got = fd.get('k');
  console.log(got instanceof File, got.name, got.size, got.type);
})();
