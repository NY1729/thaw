function def(o, name, v) {
  Object.defineProperty(o, name, { value: v, writable: true, enumerable: false, configurable: true });
}
function bytesOf(p) {
  if (p instanceof Blob) return p._bytes;
  if (p instanceof ArrayBuffer) return new Uint8Array(p.slice(0));
  if (ArrayBuffer.isView(p)) return new Uint8Array(p.buffer.slice(p.byteOffset, p.byteOffset + p.byteLength));
  return new TextEncoder().encode(String(p));
}
function Blob(parts, options) {
  if (!(this instanceof Blob)) throw new TypeError("Class constructor Blob cannot be invoked without 'new'");
  var chunks = [], n = 0, i;
  if (parts !== undefined) for (i = 0; i < parts.length; i++) { var c = bytesOf(parts[i]); chunks.push(c); n += c.length; }
  var out = new Uint8Array(n), at = 0;
  for (i = 0; i < chunks.length; i++) { out.set(chunks[i], at); at += chunks[i].length; }
  def(this, '_bytes', out);
  def(this, '_type', options && options.type !== undefined ? String(options.type).toLowerCase() : '');
}
Object.defineProperty(Blob.prototype, 'size', { get: function () { return this._bytes.length; }, configurable: true });
Object.defineProperty(Blob.prototype, 'type', { get: function () { return this._type; }, configurable: true });
def(Blob.prototype, 'slice', function (start, end, type) {
  var b = new Blob([this._bytes.slice(start, end)], { type: type });
  return b;
});
def(Blob.prototype, 'text', function () { return Promise.resolve(new TextDecoder().decode(this._bytes)); });
def(Blob.prototype, 'arrayBuffer', function () { return Promise.resolve(this._bytes.slice().buffer); });
def(Blob.prototype, 'bytes', function () { return Promise.resolve(this._bytes.slice()); });
Object.defineProperty(Blob.prototype, Symbol.toStringTag, { value: 'Blob', configurable: true });
function File(parts, name, options) {
  if (!(this instanceof File)) throw new TypeError("Class constructor File cannot be invoked without 'new'");
  if (arguments.length < 2) throw new TypeError("Failed to construct 'File': 2 arguments required, but only " + arguments.length + ' present.');
  Blob.call(this, parts, options);
  def(this, '_name', String(name));
  def(this, '_mtime', options && options.lastModified !== undefined ? Number(options.lastModified) : Date.now());
}
File.prototype = Object.create(Blob.prototype);
def(File.prototype, 'constructor', File);
Object.defineProperty(File.prototype, 'name', { get: function () { return this._name; }, configurable: true });
Object.defineProperty(File.prototype, 'lastModified', { get: function () { return this._mtime; }, configurable: true });
Object.defineProperty(File.prototype, Symbol.toStringTag, { value: 'File', configurable: true });
def(globalThis, 'Blob', Blob);
def(globalThis, 'File', File);
