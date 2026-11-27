import assert from 'assert';
import common from './index.ts';
const kLimit = Symbol('limit');
const kCallback = Symbol('callback');

class Countdown {
  [kLimit]: any; [kCallback]: any;
  constructor(limit: any, cb: any) {
    assert.strictEqual(typeof limit, 'number');
    assert.strictEqual(typeof cb, 'function');
    this[kLimit] = limit;
    this[kCallback] = common.mustCall(cb);
  }

  dec() {
    assert(this[kLimit] > 0, 'Countdown expired');
    if (--this[kLimit] === 0)
      this[kCallback]();
    return this[kLimit];
  }

  get remaining() {
    return this[kLimit];
  }
}

export default Countdown;
