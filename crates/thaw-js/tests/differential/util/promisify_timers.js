// util.promisify.custom on the timer globals. The plain thaw-js-node runtime has no require(), so the
// well-known symbol is used directly (util.promisify(setTimeout) === setTimeout[custom] is in the registry tests).
const custom = Symbol.for('nodejs.util.promisify.custom');
const d = Object.getOwnPropertyDescriptor(setTimeout, custom);
console.log('types', typeof setTimeout[custom], typeof setImmediate[custom], typeof setInterval[custom]);
console.log('flags', typeof d.get, d.set, d.enumerable, d.configurable, 'value' in d);
console.log('stable', setTimeout[custom] === setTimeout[custom], setImmediate[custom] === setImmediate[custom]);
(async () => {
  console.log('timeout', await setTimeout[custom](1, 'v'));
  console.log('immediate', await setImmediate[custom]('w'));
  const ac = new AbortController();
  const p = setTimeout[custom](1000, 'x', { signal: ac.signal });
  ac.abort();
  try { await p; } catch (e) { console.log('abort', e.name, e.code, e.message); }
  console.log('done');
})();
