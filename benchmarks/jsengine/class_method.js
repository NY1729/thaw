// 1e6 calls to a class method; expected checksum: class_method 1500000
class Counter {
  constructor() {
    this.n = 0;
  }
  add(d) {
    this.n += d;
    return this;
  }
  get value() {
    return this.n;
  }
}
const c = new Counter();
for (let i = 0; i < 1000000; i++) {
  c.add(i & 3);
}
console.log("class_method " + c.value);
