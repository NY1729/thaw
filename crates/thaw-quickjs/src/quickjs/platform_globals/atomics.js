// quickjs-ng does not yet expose Atomics.waitAsync. Keep waiter lists by
// shared data block and byte offset, which also lets a different view of the
// same SharedArrayBuffer notify a waiter.
if (typeof Atomics === 'object' && typeof Atomics.waitAsync !== 'function') {
  const waiterLists = new WeakMap();
  const nativeNotify = Atomics.notify;

  function location(typedArray, index) {
    const isInt32 = typedArray instanceof Int32Array;
    const isBigInt64 = typeof BigInt64Array === 'function' && typedArray instanceof BigInt64Array;
    if ((!isInt32 && !isBigInt64) || !(typedArray.buffer instanceof SharedArrayBuffer)) {
      throw new TypeError('Atomics.waitAsync requires a shared Int32Array or BigInt64Array');
    }
    const element = Number(index);
    if (!Number.isInteger(element) || element < 0 || element >= typedArray.length) {
      throw new RangeError('Atomics.waitAsync index is out of range');
    }
    return [typedArray.buffer, typedArray.byteOffset + element * typedArray.BYTES_PER_ELEMENT, element];
  }

  function listFor(buffer, offset, create) {
    let offsets = waiterLists.get(buffer);
    if (!offsets && create) {
      offsets = new Map();
      waiterLists.set(buffer, offsets);
    }
    let list = offsets && offsets.get(offset);
    if (!list && create) {
      list = [];
      offsets.set(offset, list);
    }
    return list;
  }

  Atomics.waitAsync = function (typedArray, index, value, timeout) {
    const [buffer, offset, element] = location(typedArray, index);
    if (Atomics.load(typedArray, element) !== value) {
      return { async: false, value: 'not-equal' };
    }
    let delay = timeout === undefined ? Infinity : Number(timeout);
    if (Number.isNaN(delay)) delay = Infinity;
    delay = Math.max(0, delay);
    if (delay === 0) return { async: false, value: 'timed-out' };

    let settle;
    const valuePromise = new Promise(resolve => { settle = resolve; });
    const waiter = { active: true, settle, timer: undefined };
    listFor(buffer, offset, true).push(waiter);
    if (Number.isFinite(delay)) {
      waiter.timer = setTimeout(() => {
        if (!waiter.active) return;
        waiter.active = false;
        settle('timed-out');
      }, delay);
    }
    return { async: true, value: valuePromise };
  };

  Atomics.notify = function (typedArray, index, count) {
    const [buffer, offset, element] = location(typedArray, index);
    let remaining = count === undefined ? Infinity : Math.max(0, Math.trunc(Number(count)));
    if (Number.isNaN(remaining)) remaining = 0;
    const list = listFor(buffer, offset, false);
    let notified = 0;
    if (list && remaining > 0) {
      for (const waiter of list) {
        if (!waiter.active || notified >= remaining) continue;
        waiter.active = false;
        if (waiter.timer !== undefined) clearTimeout(waiter.timer);
        waiter.settle('ok');
        notified++;
      }
      const active = list.filter(waiter => waiter.active);
      const offsets = waiterLists.get(buffer);
      if (active.length) offsets.set(offset, active);
      else offsets.delete(offset);
    }
    const nativeCount = nativeNotify.call(Atomics, typedArray, element, count);
    return nativeCount + notified;
  };
}
