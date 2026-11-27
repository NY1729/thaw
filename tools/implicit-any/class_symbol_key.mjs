const kLimit = Symbol('limit');
class Countdown {
  [kLimit];
  constructor(limit) { this[kLimit] = limit; }
  dec() { return --this[kLimit]; }
  get remaining() { return this[kLimit]; }
}
const c = new Countdown(2);
console.log(c.dec(), c.remaining, c.dec());
