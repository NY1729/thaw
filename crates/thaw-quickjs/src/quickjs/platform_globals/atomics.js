// `Atomics.waitAsync` is newer than the bundled quickjs-ng. Provide a
// spec-shaped approximation: a value mismatch is reported synchronously as
// `not-equal`, and a would-block wait resolves as `"timed-out"` (thaw has
// no cross-thread notification to wait on).
if (typeof Atomics === 'object' && typeof Atomics.waitAsync !== 'function') {
  Atomics.waitAsync = function (typedArray, index, value, timeout) {
    const current = Atomics.load(typedArray, index);
    if (current !== value) {
      return { async: false, value: 'not-equal' };
    }
    return { async: true, value: Promise.resolve('timed-out') };
  };
}
