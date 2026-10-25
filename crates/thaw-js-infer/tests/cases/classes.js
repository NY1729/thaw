class Vec {
  constructor(x, y) {
    this.x = x;
    this.y = y;
    this.name = 'v';
  }
  len2() { return this.x * this.x + this.y * this.y; }
  scale(k) { return new Vec(this.x * k, this.y * k); }
  label() { return this.name + '(' + this.x + ',' + this.y + ')'; }
}
var v = new Vec(3, 4);
console.log(v.len2(), v.label(), v.scale(2).label());
