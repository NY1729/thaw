// `Navigator` / `navigator` (Node 22 shape). Illegal-constructor class with enumerable prototype
// getters and no Symbol.toStringTag (Node 22 prints `Navigator {}` and `[object Object]`).
// Known gap: Node defines the global `navigator` as a lazy accessor (get/set, enumerable, configurable);
// here it is a data property, because thaw-js bare-identifier lookup does not see accessor globals yet.
const kToken = {};
function illegal() {
  const e = new TypeError('Illegal constructor');
  e.code = 'ERR_ILLEGAL_CONSTRUCTOR';
  throw e;
}
const languages = Object.freeze(['en-US']);
function platform() {
  const p = process.platform;
  const arch = process.arch;
  if (p === 'darwin') return 'MacIntel';
  if (p === 'win32') return 'Win32';
  if (p === 'linux') {
    if (arch === 'ia32') return 'Linux i686';
    if (arch === 'x64') return 'Linux x86_64';
    if (arch === 'arm64') return 'Linux aarch64';
    return 'Linux ' + arch;
  }
  return p + ' ' + arch;
}
class Navigator {
  constructor(...args) {
    if (args[0] !== kToken) illegal();
  }
}
// Getters come from an object literal so their .name is "get <key>" as in Node. Defined after the class
// so `constructor` stays the first prototype key.
const getters = {
  get hardwareConcurrency() { return __thaw_available_parallelism(); },
  get language() { return languages[0]; },
  get languages() { return languages; },
  get userAgent() { return 'Node.js/22'; },
  get platform() { return platform(); },
};
for (const name of Object.keys(getters)) {
  Object.defineProperty(Navigator.prototype, name, { get: Object.getOwnPropertyDescriptor(getters, name).get, enumerable: true, configurable: true });
}
Object.defineProperty(globalThis, 'Navigator', { value: Navigator, writable: true, enumerable: false, configurable: true });
Object.defineProperty(globalThis, 'navigator', { value: new Navigator(kToken), writable: true, enumerable: true, configurable: true });
